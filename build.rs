use anyhow::{Result, anyhow};
use std::{
    io::Read,
    process::{Command, Stdio},
};

fn run_command(cmd: &str) -> Result<String> {
    let mut child = Command::new("sh")
        .arg("-c")
        .arg(cmd)
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()?;

    let status = child.wait()?;

    if status.success() {
        let mut buf = Vec::new();
        child.stdout.unwrap().read_to_end(&mut buf)?;
        let output = String::from_utf8_lossy(&buf);
        Ok(output.to_string())
    } else {
        Err(anyhow!("Command {} failed with status: {:?}", cmd, status))
    }
}

const GET_VERSION_COMMAND: &str = r#"\
printf "$(grep -E '^version' Cargo.toml | cut -d '"' -f2)."
printf $(git rev-parse --short=7 HEAD 2>/dev/null);
if ! git diff-index --quiet HEAD --; then 
    printf '+' 
fi
date +.%y%m%d-%H%M%S"#;

fn main() {
    // re-build if this file is changed
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=src/");

    // remove plot only mode due to performance issue
    //println!("cargo:rustc-env=CONFIG_PLOT_ONLY_MODE_ENABLED=0");

    let version = run_command(GET_VERSION_COMMAND).unwrap();
    println!("cargo:rustc-env=VERSION={}", version);
}
