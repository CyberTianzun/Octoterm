use octoterm_desktop::configfile::{save, Editable};

fn edit(listen: &str, token: Option<&str>) -> Editable {
    Editable { listen: listen.parse().unwrap(), token: token.map(String::from) }
}

#[test]
fn preserves_comments_and_launcher_sections() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    std::fs::write(
        &path,
        r#"# 我手写的注释,不许动
listen = "127.0.0.1:7683"

# 启动项
[[launcher]]
name = "prod ssh"
command = ["ssh", "prod01"]
"#,
    )
    .unwrap();

    save(&path, &edit("127.0.0.1:9000", None)).unwrap();

    let out = std::fs::read_to_string(&path).unwrap();
    assert!(out.contains("# 我手写的注释,不许动"), "注释被碾掉了:\n{out}");
    assert!(out.contains("# 启动项"), "段注释被碾掉了:\n{out}");
    assert!(out.contains(r#"name = "prod ssh""#), "launcher 段丢了:\n{out}");
    assert!(out.contains(r#"listen = "127.0.0.1:9000""#), "listen 没改:\n{out}");
}

#[test]
fn creates_a_minimal_file_when_missing() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("sub").join("config.toml");

    save(&path, &edit("0.0.0.0:7683", Some("fixed"))).unwrap();

    let out = std::fs::read_to_string(&path).unwrap();
    assert!(out.contains(r#"listen = "0.0.0.0:7683""#), "{out}");
    assert!(out.contains(r#"token = "fixed""#), "{out}");
}

#[test]
fn clearing_the_token_removes_the_key() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    std::fs::write(&path, "listen = \"127.0.0.1:7683\"\ntoken = \"old\"\n").unwrap();

    save(&path, &edit("127.0.0.1:7683", None)).unwrap();

    let out = std::fs::read_to_string(&path).unwrap();
    assert!(!out.contains("token"), "token 键应当被移除:\n{out}");
}

#[test]
fn a_broken_file_is_reported_and_left_untouched() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    let broken = "listen = = =\n";
    std::fs::write(&path, broken).unwrap();

    assert!(save(&path, &edit("127.0.0.1:9000", None)).is_err());
    assert_eq!(std::fs::read_to_string(&path).unwrap(), broken, "坏文件不该被改写");
}

#[test]
fn no_temp_file_is_left_behind() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");

    save(&path, &edit("127.0.0.1:7683", None)).unwrap();

    let leftovers: Vec<_> = std::fs::read_dir(dir.path())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|n| n.ends_with(".tmp"))
        .collect();
    assert!(leftovers.is_empty(), "残留临时文件:{leftovers:?}");
}
