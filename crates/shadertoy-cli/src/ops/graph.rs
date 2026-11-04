use super::*;
use crate::manifest::{FrameRef, InputKind, PassKind};
use crate::source::expand_all;
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

#[derive(Debug, Clone)]
pub struct GraphOptions {
    pub project: PathBuf,
    pub preset: Option<String>,
    pub dot: Option<PathBuf>,
}

#[derive(Debug, Clone, Serialize)]
pub struct GraphDiagnostic {
    pub level: &'static str,
    pub code: &'static str,
    pub message: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub passes: Vec<String>,
}

impl GraphDiagnostic {
    fn error(code: &'static str, message: impl Into<String>, passes: Vec<String>) -> Self {
        Self {
            level: "error",
            code,
            message: message.into(),
            passes,
        }
    }

    fn warning(code: &'static str, message: impl Into<String>, passes: Vec<String>) -> Self {
        Self {
            level: "warning",
            code,
            message: message.into(),
            passes,
        }
    }
}

pub fn graph_project(options: &GraphOptions) -> Result<Output> {
    let loaded = LoadedManifest::load_with_preset(&options.project, options.preset.as_deref())?;
    ensure_source_files_exist(&loaded)?;
    let diagnostics = analyze_graph(&loaded, true)?;

    let edges = pass_edges(&loaded)?;
    let dot = render_dot(&loaded, &edges);
    if let Some(path) = &options.dot {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, &dot)
            .with_context(|| format!("failed to write graph DOT {}", path.display()))?;
    }

    let mut human = format!("Project graph: {}\n", loaded.manifest.project.name);
    for pass in &loaded.manifest.passes {
        let (width, height) = loaded.manifest.pass_dimensions(
            pass,
            loaded.manifest.render.width,
            loaded.manifest.render.height,
        );
        human.push_str(&format!(
            "  {} ({:?}, {}x{})\n",
            pass.name, pass.kind, width, height
        ));
        for input in &pass.inputs {
            let kind = loaded.manifest.infer_input_kind(input)?;
            human.push_str(&format!(
                "    iChannel{} <- {} ({:?}, {:?}, output {})\n",
                input.channel, input.source, kind, input.frame, input.output
            ));
        }
        for storage in &pass.storage {
            human.push_str(&format!(
                "    SSBO binding {} <-> {} ({} bytes)\n",
                storage.binding, storage.name, storage.size
            ));
        }
    }
    append_diagnostics(&mut human, &diagnostics);
    if let Some(path) = &options.dot {
        human.push_str(&format!("DOT: {}\n", path.display()));
    }

    let errors = diagnostics.iter().filter(|d| d.level == "error").count();
    let warnings = diagnostics.iter().filter(|d| d.level == "warning").count();
    Ok(Output {
        human: human.trim_end().to_string(),
        json: json!({
            "ok": errors == 0,
            "project": loaded.manifest.project.name,
            "preset": options.preset,
            "passes": loaded.manifest.passes,
            "edges": edges,
            "diagnostics": diagnostics,
            "errors": errors,
            "warnings": warnings,
            "dot": options.dot,
        }),
    })
}

pub(super) fn analyze_graph(
    loaded: &LoadedManifest,
    include_pedantic: bool,
) -> Result<Vec<GraphDiagnostic>> {
    let manifest = &loaded.manifest;
    let mut diagnostics = Vec::new();
    let pass_index = manifest
        .passes
        .iter()
        .enumerate()
        .map(|(index, pass)| (pass.name.as_str(), index))
        .collect::<HashMap<_, _>>();

    let mut current_adjacency = vec![Vec::<usize>::new(); manifest.passes.len()];
    let mut reverse_all = vec![Vec::<usize>::new(); manifest.passes.len()];
    for (consumer_index, pass) in manifest.passes.iter().enumerate() {
        for input in &pass.inputs {
            if manifest.infer_input_kind(input)? != InputKind::Pass {
                continue;
            }
            let source_index = *pass_index
                .get(input.source.as_str())
                .expect("manifest validation guarantees pass source exists");
            reverse_all[consumer_index].push(source_index);
            if input.frame == FrameRef::Current {
                current_adjacency[source_index].push(consumer_index);
            }
        }
    }

    if let Some(cycle) = find_cycle(&current_adjacency) {
        let names = cycle
            .iter()
            .map(|index| manifest.passes[*index].name.clone())
            .collect::<Vec<_>>();
        diagnostics.push(GraphDiagnostic::error(
            "current-frame-cycle",
            format!("current-frame dependency cycle: {}", names.join(" -> ")),
            names,
        ));
    }

    if include_pedantic {
        let final_index = *pass_index
            .get(manifest.final_pass().name.as_str())
            .expect("final pass is in manifest");
        let mut reachable = HashSet::new();
        let mut stack = vec![final_index];
        while let Some(index) = stack.pop() {
            if !reachable.insert(index) {
                continue;
            }
            stack.extend(reverse_all[index].iter().copied());
        }
        for (index, pass) in manifest.passes.iter().enumerate() {
            if pass.kind != PassKind::Sound && !reachable.contains(&index) {
                diagnostics.push(GraphDiagnostic::warning(
                    "unreachable-pass",
                    format!(
                        "pass '{}' does not contribute to the final Image pass",
                        pass.name
                    ),
                    vec![pass.name.clone()],
                ));
            }
        }

        let used_assets = manifest
            .passes
            .iter()
            .flat_map(|pass| &pass.inputs)
            .filter_map(|input| {
                manifest
                    .infer_input_kind(input)
                    .ok()
                    .filter(|kind| *kind != InputKind::Pass)
                    .map(|_| input.source.as_str())
            })
            .collect::<HashSet<_>>();
        for asset in &manifest.assets {
            if !used_assets.contains(asset.name.as_str()) {
                diagnostics.push(GraphDiagnostic::warning(
                    "unused-asset",
                    format!("asset '{}' is declared but never bound", asset.name),
                    Vec::new(),
                ));
            }
        }

        let feedback_sources = manifest
            .passes
            .iter()
            .flat_map(|pass| &pass.inputs)
            .filter(|input| input.frame == FrameRef::Previous)
            .map(|input| input.source.as_str())
            .collect::<HashSet<_>>();
        for pass in &manifest.passes {
            if matches!(
                pass.kind,
                PassKind::Buffer | PassKind::Compute | PassKind::Mesh
            ) && pass.width.is_none()
                && feedback_sources.contains(pass.name.as_str())
            {
                diagnostics.push(GraphDiagnostic::warning(
                    "viewport-sized-feedback",
                    format!(
                        "feedback pass '{}' inherits output resolution; changing preview/render size also changes its persistent state resolution",
                        pass.name
                    ),
                    vec![pass.name.clone()],
                ));
            }
        }

        let reachability = transitive_reachability(&current_adjacency);
        let mut storage_users: BTreeMap<&str, BTreeSet<usize>> = BTreeMap::new();
        for (index, pass) in manifest.passes.iter().enumerate() {
            for storage in &pass.storage {
                storage_users
                    .entry(storage.name.as_str())
                    .or_default()
                    .insert(index);
            }
        }
        for (name, users) in storage_users {
            let users = users.into_iter().collect::<Vec<_>>();
            for left in 0..users.len() {
                for right in (left + 1)..users.len() {
                    let a = users[left];
                    let b = users[right];
                    if !reachability[a].contains(&b) && !reachability[b].contains(&a) {
                        let a_name = manifest.passes[a].name.clone();
                        let b_name = manifest.passes[b].name.clone();
                        diagnostics.push(GraphDiagnostic::warning(
                            "unordered-shared-storage",
                            format!(
                                "shared SSBO '{}' is used by '{}' and '{}' without a current-frame dependency ordering them",
                                name, a_name, b_name
                            ),
                            vec![a_name, b_name],
                        ));
                    }
                }
            }
        }

        let sources = expand_all(loaded)?;
        for definition in &manifest.uniforms {
            let name = definition.name();
            if !sources
                .values()
                .any(|source| contains_identifier(&source.text, name))
            {
                diagnostics.push(GraphDiagnostic::warning(
                    "unused-uniform",
                    format!(
                        "custom uniform '{}' is declared but not referenced by shader source",
                        name
                    ),
                    Vec::new(),
                ));
            }
        }
    }

    Ok(diagnostics)
}

fn pass_edges(loaded: &LoadedManifest) -> Result<Vec<serde_json::Value>> {
    let mut edges = Vec::new();
    for pass in &loaded.manifest.passes {
        for input in &pass.inputs {
            let kind = loaded.manifest.infer_input_kind(input)?;
            edges.push(json!({
                "from": input.source,
                "to": pass.name,
                "channel": input.channel,
                "kind": format!("{kind:?}").to_lowercase(),
                "frame": format!("{:?}", input.frame).to_lowercase(),
                "output": input.output,
                "filter": format!("{:?}", input.filter).to_lowercase(),
                "wrap": format!("{:?}", input.wrap).to_lowercase(),
            }));
        }
    }
    Ok(edges)
}

fn render_dot(loaded: &LoadedManifest, edges: &[serde_json::Value]) -> String {
    let mut dot = String::from("digraph ShaderToy {\n  rankdir=LR;\n");
    for pass in &loaded.manifest.passes {
        dot.push_str(&format!(
            "  \"{}\" [shape=box,label=\"{}\\n{:?}\"];\n",
            dot_escape(&pass.name),
            dot_escape(&pass.name),
            pass.kind
        ));
    }
    for asset in &loaded.manifest.assets {
        dot.push_str(&format!(
            "  \"asset:{}\" [shape=ellipse,label=\"{}\\nasset\"];\n",
            dot_escape(&asset.name),
            dot_escape(&asset.name)
        ));
    }
    let mut storages = BTreeSet::new();
    for pass in &loaded.manifest.passes {
        for storage in &pass.storage {
            storages.insert(storage.name.as_str());
        }
    }
    for storage in storages {
        dot.push_str(&format!(
            "  \"storage:{}\" [shape=diamond,label=\"{}\\nSSBO\"];\n",
            dot_escape(storage),
            dot_escape(storage)
        ));
    }
    for edge in edges {
        let Some(from) = edge.get("from").and_then(serde_json::Value::as_str) else {
            continue;
        };
        let Some(to) = edge.get("to").and_then(serde_json::Value::as_str) else {
            continue;
        };
        let kind = edge
            .get("kind")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("");
        let frame = edge
            .get("frame")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("");
        let channel = edge
            .get("channel")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0);
        let source = if kind == "pass" {
            from.to_string()
        } else {
            format!("asset:{from}")
        };
        let style = if frame == "previous" {
            "dashed"
        } else {
            "solid"
        };
        dot.push_str(&format!(
            "  \"{}\" -> \"{}\" [label=\"iChannel{}\",style={}];\n",
            dot_escape(&source),
            dot_escape(to),
            channel,
            style
        ));
    }
    for pass in &loaded.manifest.passes {
        for storage in &pass.storage {
            dot.push_str(&format!(
                "  \"{}\" -> \"storage:{}\" [dir=both,label=\"binding {}\",color=gray50];\n",
                dot_escape(&pass.name),
                dot_escape(&storage.name),
                storage.binding
            ));
        }
    }
    dot.push_str("}\n");
    dot
}

fn append_diagnostics(human: &mut String, diagnostics: &[GraphDiagnostic]) {
    if diagnostics.is_empty() {
        human.push_str("Diagnostics: none\n");
        return;
    }
    human.push_str("Diagnostics:\n");
    for diagnostic in diagnostics {
        human.push_str(&format!(
            "  {} [{}] {}\n",
            diagnostic.level.to_ascii_uppercase(),
            diagnostic.code,
            diagnostic.message
        ));
    }
}

fn find_cycle(adjacency: &[Vec<usize>]) -> Option<Vec<usize>> {
    fn visit(
        node: usize,
        adjacency: &[Vec<usize>],
        state: &mut [u8],
        stack: &mut Vec<usize>,
    ) -> Option<Vec<usize>> {
        state[node] = 1;
        stack.push(node);
        for &next in &adjacency[node] {
            match state[next] {
                0 => {
                    if let Some(cycle) = visit(next, adjacency, state, stack) {
                        return Some(cycle);
                    }
                }
                1 => {
                    let start = stack.iter().position(|item| *item == next).unwrap_or(0);
                    let mut cycle = stack[start..].to_vec();
                    cycle.push(next);
                    return Some(cycle);
                }
                _ => {}
            }
        }
        stack.pop();
        state[node] = 2;
        None
    }

    let mut state = vec![0u8; adjacency.len()];
    let mut stack = Vec::new();
    for node in 0..adjacency.len() {
        if state[node] == 0
            && let Some(cycle) = visit(node, adjacency, &mut state, &mut stack)
        {
            return Some(cycle);
        }
    }
    None
}

fn transitive_reachability(adjacency: &[Vec<usize>]) -> Vec<HashSet<usize>> {
    let mut result = Vec::with_capacity(adjacency.len());
    for start in 0..adjacency.len() {
        let mut seen = HashSet::new();
        let mut stack = adjacency[start].clone();
        while let Some(node) = stack.pop() {
            if seen.insert(node) {
                stack.extend(adjacency[node].iter().copied());
            }
        }
        result.push(seen);
    }
    result
}

fn contains_identifier(text: &str, identifier: &str) -> bool {
    if identifier.is_empty() {
        return false;
    }
    let bytes = text.as_bytes();
    let needle = identifier.as_bytes();
    if needle.len() > bytes.len() {
        return false;
    }
    for start in 0..=(bytes.len() - needle.len()) {
        if &bytes[start..start + needle.len()] != needle {
            continue;
        }
        let left_ok = start == 0 || !is_identifier_byte(bytes[start - 1]);
        let end = start + needle.len();
        let right_ok = end == bytes.len() || !is_identifier_byte(bytes[end]);
        if left_ok && right_ok {
            return true;
        }
    }
    false
}

fn is_identifier_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

fn dot_escape(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identifier_search_respects_boundaries() {
        assert!(contains_identifier("float foo = 1.0;", "foo"));
        assert!(!contains_identifier("float foobar = 1.0;", "foo"));
    }

    #[test]
    fn cycle_detection_returns_closed_cycle() {
        let cycle = find_cycle(&[vec![1], vec![2], vec![0]]).expect("cycle");
        assert_eq!(cycle.first(), cycle.last());
    }
}
