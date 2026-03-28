use std::{
    env, fs,
    path::{Path, PathBuf},
};

pub fn resolve_cli_cwd() -> std::io::Result<PathBuf> {
    let current_dir = env::current_dir()?;
    Ok(prefer_lexical_pwd(
        current_dir,
        env::var_os("PWD").as_deref().map(Path::new),
    ))
}

fn prefer_lexical_pwd(current_dir: PathBuf, pwd: Option<&Path>) -> PathBuf {
    let Some(pwd) = pwd else {
        return current_dir;
    };
    if !pwd.is_absolute() {
        return current_dir;
    }
    match (fs::canonicalize(&current_dir), fs::canonicalize(pwd)) {
        (Ok(actual), Ok(lexical)) if actual == lexical => pwd.to_path_buf(),
        _ => current_dir,
    }
}

#[cfg(test)]
mod tests {
    use super::prefer_lexical_pwd;
    use std::{
        fs,
        path::{Path, PathBuf},
    };
    use tempfile::tempdir;

    #[cfg(unix)]
    use std::os::unix::fs::symlink;

    #[test]
    fn falls_back_when_pwd_is_missing_or_relative() {
        let current_dir = PathBuf::from("/tmp/current");
        assert_eq!(prefer_lexical_pwd(current_dir.clone(), None), current_dir);
        assert_eq!(
            prefer_lexical_pwd(current_dir.clone(), Some(Path::new("relative/path"))),
            current_dir
        );
    }

    #[test]
    fn falls_back_when_pwd_points_elsewhere() {
        let dir = tempdir().expect("tempdir");
        let current_dir = dir.path().join("current");
        let other_dir = dir.path().join("other");
        fs::create_dir_all(&current_dir).expect("create current");
        fs::create_dir_all(&other_dir).expect("create other");

        assert_eq!(
            prefer_lexical_pwd(current_dir.clone(), Some(other_dir.as_path())),
            current_dir
        );
    }

    #[cfg(unix)]
    #[test]
    fn prefers_absolute_pwd_when_it_resolves_to_current_dir() {
        let dir = tempdir().expect("tempdir");
        let physical = dir.path().join("physical");
        let lexical = dir.path().join("lexical-link");
        fs::create_dir_all(&physical).expect("create physical");
        symlink(&physical, &lexical).expect("symlink lexical cwd");

        let current_dir = fs::canonicalize(&lexical).expect("canonical current");
        assert_eq!(
            prefer_lexical_pwd(current_dir, Some(lexical.as_path())),
            lexical
        );
    }
}
