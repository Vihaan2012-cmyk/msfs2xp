use std::process::Command;

#[test]
fn help_lists_subcommands() {
    let out = Command::new(env!("CARGO_BIN_EXE_msfs2xp"))
        .arg("--help")
        .output()
        .unwrap();
    let s = String::from_utf8_lossy(&out.stdout);
    for cmd in ["convert", "list", "inspect", "preview", "validate"] {
        assert!(s.contains(cmd), "help output missing `{cmd}`:\n{s}");
    }
}
