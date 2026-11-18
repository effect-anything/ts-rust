//! Not in tsgo. A mapper package whose `exec` is one launcher (`./bin/<name>`)
//! may ship that program natively in a per-platform package,
//! `<package>-<platform>-<arch>/lib/<name>`, as npm packages with optional
//! platform dependencies do. When that file is installed it replaces `exec`,
//! so a mapper start does not pay for a Node process. Nothing in the manifest
//! names it: this is a host convention.
//!
//! `frontend/tsoptions/content_mappers.rs` calls [`exec`].

use crate::frontend::prelude::*;

/// `exec`, or the installed native program that replaces it.
pub fn exec(
    host: &dyn ParseConfigHost,
    resolver: &DefaultResolver,
    package_directory: &str,
    package_name: &str,
    exec: Vec<String>,
) -> Vec<String> {
    native_exec(host, resolver, package_directory, package_name, &exec)
        .map_or(exec, |path| vec![path])
}

/// Node's `process.platform` and `process.arch` for this build.
fn node_platform() -> (&'static str, &'static str) {
    let platform = match std::env::consts::OS {
        "macos" => "darwin",
        "windows" => "win32",
        other => other,
    };
    let arch = match std::env::consts::ARCH {
        "aarch64" => "arm64",
        "x86_64" => "x64",
        "x86" => "ia32",
        other => other,
    };
    (platform, arch)
}

/// The native program for a one-entry `exec` (`[".../<name>"]`):
/// `<package>-<platform>-<arch>/lib/<name>` (Node's spelling of platform
/// and arch), resolved from the mapper's package directory, when that file
/// exists; `.exe` is tried on Windows. `None` keeps `exec`.
pub(crate) fn native_exec(
    host: &dyn ParseConfigHost,
    resolver: &DefaultResolver,
    package_directory: &str,
    package_name: &str,
    exec: &[String],
) -> Option<String> {
    let [launcher] = exec else {
        return None;
    };
    let program = launcher
        .rsplit(['/', '\\'])
        .next()
        .filter(|name| !name.is_empty() && *name != "..")?;
    let (platform, arch) = node_platform();
    let package = format!("{package_name}-{platform}-{arch}");
    let containing_file = combine_paths(package_directory, &["package.json"]);
    let resolved = resolver
        .resolve_package_directory(&package, &containing_file, RESOLUTION_MODE_NONE, None)
        .filter(|resolved| !resolved.resolved_file_name.is_empty())?;
    let path = combine_paths(&resolved.resolved_file_name, &["lib", program]);
    let fs = host.fs();
    if fs.file_exists(&path) {
        return Some(path);
    }
    let exe = format!("{path}.exe");
    (platform == "win32" && fs.file_exists(&exe)).then_some(exe)
}

#[cfg(test)]
mod native_exec_tests {
    use super::*;
    use crate::frontend::tsoptions::content_mappers::resolve_content_mapper_manifest;
    use crate::frontend::vfs::osvfs_fs;

    struct Host(String);

    impl ParseConfigHost for Host {
        fn fs(&self) -> Rc<dyn Fs> {
            osvfs_fs()
        }
        fn get_current_directory(&self) -> String {
            self.0.clone()
        }
    }

    fn write(path: &std::path::Path, text: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    /// An installed platform package replaces `exec`; a missing one keeps it.
    #[test]
    fn an_installed_platform_package_replaces_the_launcher() {
        let root =
            std::env::temp_dir().join(format!("goport-platform-exec-{}", std::process::id()));
        let root = std::path::PathBuf::from(os_fs_realpath_dir(&root));
        let (platform, arch) = node_platform();
        let modules = root.join("node_modules");
        write(
            &modules.join("@x/m/package.json"),
            r#"{"name":"@x/m","version":"1.0.0","typescript":{"contentMapper":{"exec":["./bin/m"]}}}"#,
        );
        write(
            &modules.join("missing/package.json"),
            r#"{"name":"missing","typescript":{"contentMapper":{"exec":["./bin/m"]}}}"#,
        );
        let native = modules.join(format!("@x/m-{platform}-{arch}/lib/m"));
        write(
            &modules.join(format!("@x/m-{platform}-{arch}/package.json")),
            &format!(r#"{{"name":"@x/m-{platform}-{arch}"}}"#),
        );
        write(&native, "");
        let root_text = root.to_str().unwrap().replace('\\', "/");
        let host = Host(root_text.clone());
        let containing = format!("{root_text}/tsconfig.json");

        let (manifest, _, diagnostic) = resolve_content_mapper_manifest(&host, &containing, "@x/m");
        assert!(diagnostic.is_none());
        assert_eq!(
            manifest.exec,
            vec![format!(
                "{root_text}/node_modules/@x/m-{platform}-{arch}/lib/m"
            )]
        );

        let (manifest, _, diagnostic) =
            resolve_content_mapper_manifest(&host, &containing, "missing");
        assert!(diagnostic.is_none());
        assert_eq!(manifest.exec, vec!["./bin/m".to_string()]);
        let _ = std::fs::remove_dir_all(&root);
    }

    /// The temp directory's real path (`/var` is `/private/var` on macOS),
    /// as package resolution returns real paths.
    fn os_fs_realpath_dir(path: &std::path::Path) -> String {
        std::fs::create_dir_all(path).unwrap();
        std::fs::canonicalize(path)
            .unwrap()
            .to_string_lossy()
            .into_owned()
    }
}
