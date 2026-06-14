fn main() {
    let version =
        match std::process::Command::new("git").arg("describe").arg("--long").arg("--abbrev=7").arg("--tags").output() {
            // `git describe --tags` fails (exit != 0, empty stdout) when the
            // checkout has no tags — e.g. a shallow clone or a fork that
            // never fetched them. Guard on the exit status and the expected
            // `<tag>-<rev>-<hash>` shape instead of blindly indexing, which
            // panicked with "byte index 1 out of bounds" on empty output.
            Ok(output) if output.status.success() => {
                let str = String::from_utf8_lossy(&output.stdout);
                match str.trim().split('-').collect::<Vec<_>>().as_slice() {
                    [tag, rev, hash] => format!("{}-r{}-{}", tag.strip_prefix('v').unwrap_or(tag), rev, hash),
                    _ => String::from("unknown"),
                }
            }
            Ok(_) => String::from("unknown"),
            Err(err) => {
                println!("cargo::warning=Unable to get git version: {err:#}");
                String::from("unknown")
            }
        };

    let commit = match std::process::Command::new("git").arg("log").arg("--format=[%s]").arg("-n").arg("1").output() {
        Ok(output) => {
            if output.stdout.is_empty() {
                String::from("unknown")
            } else {
                String::from_utf8_lossy(&output.stdout).trim().to_string()
            }
        }
        Err(err) => {
            println!("cargo::warning=Unable to get latest git commit: {err:#}");
            String::from("unknown")
        }
    };

    println!("cargo::rustc-env=GIT_VERSION={version} {commit}");
}
