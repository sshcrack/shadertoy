mod assets;
mod audio;
mod cli;
mod docs;
mod importer;
mod manifest;
mod media;
mod ops;
mod preview;
mod project;
mod project_schema;
mod replay;
mod scaffold;
mod source;
mod state;
mod uniforms;

use std::process::ExitCode;

fn main() -> ExitCode {
    cli::run()
}
