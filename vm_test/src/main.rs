use anyhow::{bail, Result};
use std::process::Command;

fn main() -> Result<()> {
    eprintln!("Testing Sellafield using a VM");

    build_linux_release()?;

    let output = run_vm()?;

    // TODO: More robust checks.
    if !output.contains("test_output") {
        bail!("`test_output` not found in output");
    }
    if !output.contains("core.vagrant.generate_core_dump") {
        bail!("`core.vagrant.generate_core_dump` not found in output");
    }

    eprintln!("Test succeeded. Output:\n{}", output);

    Ok(())
}

fn build_linux_release() -> Result<()> {
    let status = Command::new("cargo")
        .arg("build")
        .arg("--release")
        .arg("--target")
        .arg("x86_64-unknown-linux-musl")
        .current_dir("..")
        .status()?;

    if !status.success() {
        bail!("Cargo build failed");
    }

    Ok(())
}

fn run_vm() -> Result<String> {
    std::fs::copy("../target/x86_64-unknown-linux-musl/release/sellafield", "../test_input/sellafield")?;

    // Unfortunately `--copy-out-after /home/vagrant/test_output:test_output`
    // gives a permission error that I'm not sure about but we can just scrape stdout.
    let output = Command::new("transient")
        .arg("run")
        .arg("centos/7:2004.01")
        .arg("--copy-in-before")
        .arg("test_input:/home/vagrant/test_input")
        .arg("--ssh-command")
        .arg("/home/vagrant/test_input/test.sh")
        .arg("--")
        .arg("-m")
        .arg("1G")
        .current_dir("..")
        .output()?;

    let stdout = latin1_to_string(&output.stdout);
    let stderr = latin1_to_string(&output.stderr);

    if !output.status.success() {
        bail!("Transient run failed.\n--- Stdout ---\n{}--- Stderr ---\n{}", stdout, stderr);
    }

    Ok(stdout)
}

fn latin1_to_string(s: &[u8]) -> String {
    s.iter().map(|&c| c as char).collect()
}
