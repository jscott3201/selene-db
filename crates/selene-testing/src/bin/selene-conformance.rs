//! Executable conformance evidence and traceability command.

fn main() -> Result<(), Box<dyn std::error::Error>> {
    selene_testing::conformance::main_cli()
}
