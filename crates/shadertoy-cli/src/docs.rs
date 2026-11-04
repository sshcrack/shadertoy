use crate::manifest::schema_json;
use anyhow::{Result, bail};

pub fn topic(name: &str, schema: bool) -> Result<String> {
    if schema {
        if name != "manifest" {
            bail!("--schema is only valid with 'shadertoy docs manifest'");
        }
        return schema_json();
    }

    let text = match name {
        "agent" => crate::include_file!("docs/agent.md"),
        "project" => crate::include_file!("docs/project.md"),
        "import" => crate::include_file!("docs/import.md"),
        "manifest" => crate::include_file!("docs/manifest.md"),
        "passes" => crate::include_file!("docs/passes.md"),
        "glsl" => crate::include_file!("docs/glsl.md"),
        "assets" => crate::include_file!("docs/assets.md"),
        "audio" => crate::include_file!("docs/audio.md"),
        "buffers" => crate::include_file!("docs/buffers.md"),
        "channels" => crate::include_file!("docs/channels.md"),
        "state" => crate::include_file!("docs/state.md"),
        "sweep" => crate::include_file!("docs/sweep.md"),
        "blind" => crate::include_file!("docs/blind.md"),
        "experiment" => crate::include_file!("docs/experiment.md"),
        "profile" => crate::include_file!("docs/profile.md"),
        "test" => crate::include_file!("docs/test.md"),
        "trace" => crate::include_file!("docs/trace.md"),
        "graph" => crate::include_file!("docs/graph.md"),
        "preview" => crate::include_file!("docs/preview.md"),
        other => bail!(
            "unknown documentation topic '{other}'; expected agent, project, import, manifest, passes, glsl, assets, audio, buffers, channels, state, sweep, blind, experiment, profile, test, trace, graph, or preview"
        ),
    };
    Ok(text.trim().to_string())
}
