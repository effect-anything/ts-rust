//! Go: tsoptions/contentmappers.go (tsgo#4712).

use crate::contentmapper::Manifest;
use crate::frontend::prelude::*;

// Go: tsoptions/contentmappers.go:31 resolveContentMapperManifest
// resolveContentMapperManifest locates packageName in node_modules (walking up from the directory of
// containingFile via node module resolution) and reads its package.json to produce the mapper's manifest
// and package directory. It never executes the package. On failure it returns a diagnostic describing why
// the mapper could not be resolved; on success the diagnostic is nil.
// PORT: Go nil `*ast.Diagnostic` is `None`. Go passes the `ParseConfigHost`
// as the resolution host; `ResolverHost` keeps its `FS()` and
// `GetCurrentDirectory()` values, as for `extends`.
pub fn resolve_content_mapper_manifest(
    host: &dyn ParseConfigHost,
    containing_file: &str,
    package_name: &str,
) -> (Manifest, String, Option<Diagnostic>) {
    let resolver_host: Rc<dyn ResolutionHost> = Rc::new(ResolverHost {
        fs: host.fs(),
        current_directory: host.get_current_directory(),
    });
    let resolver = new_resolver(ResolverOptions {
        host: Some(resolver_host),
        compiler_options: Some(Rc::new(CompilerOptions {
            module_resolution: ModuleResolutionKind::BUNDLER,
            ..Default::default()
        })),
        ..Default::default()
    });
    let resolved = resolver.resolve_package_directory(
        package_name,
        containing_file,
        RESOLUTION_MODE_NONE,
        None,
    );
    let Some(resolved) = resolved.filter(|resolved| !resolved.resolved_file_name.is_empty()) else {
        return (
            Manifest::default(),
            String::new(),
            Some(new_compiler_diagnostic(
                diag::The_content_mapper_package_0_could_not_be_resolved,
                args![package_name],
            )),
        );
    };
    let package_directory = resolved.resolved_file_name;

    let package_json_path = combine_paths(&package_directory, &["package.json"]);
    let (contents, ok) = host.fs().read_file(&package_json_path);
    if !ok {
        return (
            Manifest::default(),
            package_directory,
            Some(new_compiler_diagnostic(
                diag::The_content_mapper_package_0_could_not_be_resolved,
                args![package_name],
            )),
        );
    }
    // PORT: `contents` is the port form of the Go text; Go parses its bytes.
    let Ok(fields) = crate::frontend::packagejson::parse(&go_string_bytes(&contents)) else {
        return (
            Manifest::default(),
            package_directory,
            Some(new_compiler_diagnostic(
                diag::The_package_json_of_the_content_mapper_package_0_could_not_be_parsed,
                args![package_name],
            )),
        );
    };
    let (name, _) = fields.header_fields.name.get_value();
    if name.is_empty() {
        return (
            Manifest::default(),
            package_directory,
            Some(new_compiler_diagnostic(
                diag::The_package_json_of_the_content_mapper_package_0_does_not_specify_a_name,
                args![package_name],
            )),
        );
    }
    let (version, _) = fields.header_fields.version.get_value();

    // A content mapper package must declare how to run it: a "typescript.contentMapper" object with a non-empty
    // "exec" array of strings.
    let (cm, ok) = fields.content_mapper.get_value();
    if !ok {
        return (
            Manifest::default(),
            package_directory,
            Some(new_compiler_diagnostic(
                diag::The_package_json_of_the_content_mapper_package_0_does_not_declare_a_typescript_contentMapper_object,
                args![package_name],
            )),
        );
    }
    let (exec, ok) = cm.exec.get_value();
    if !ok || exec.is_empty() {
        return (
            Manifest::default(),
            package_directory,
            Some(new_compiler_diagnostic(
                diag::The_typescript_contentMapper_exec_of_the_content_mapper_package_0_must_be_a_non_empty_array_of_strings,
                args![package_name],
            )),
        );
    }
    // PORT: not in tsgo. A native executable for this platform, when the
    // package names one and it is installed, replaces `exec` (often a Node
    // launcher) so a mapper start does not pay for a Node process.
    let (platform_exec, has_platform_exec) = cm.platform_exec.get_value();
    let exec = if has_platform_exec {
        resolve_platform_exec(host, &resolver, &package_directory, &platform_exec)
            .map_or(exec, |path| vec![path])
    } else {
        exec
    };
    let (compiler_options, _) = cm.compiler_options.get_value();
    let (dynamic_config, _) = cm.dynamic_config.get_value();
    (
        Manifest {
            name,
            version,
            exec,
            compiler_options,
            dynamic_config,
        },
        package_directory,
        None,
    )
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

/// `platformExec` (`"<package>/<path>"`, `${platform}` and `${arch}` in
/// Node's spelling) resolved from the mapper's package directory to an
/// existing file; `.exe` is tried on Windows. `None` keeps `exec`.
fn resolve_platform_exec(
    host: &dyn ParseConfigHost,
    resolver: &DefaultResolver,
    package_directory: &str,
    template: &str,
) -> Option<String> {
    let (platform, arch) = node_platform();
    let spec = template
        .replace("${platform}", platform)
        .replace("${arch}", arch);
    let parts: Vec<&str> = spec.split('/').collect();
    let name_len = if spec.starts_with('@') { 2 } else { 1 };
    if parts.len() <= name_len || parts.iter().any(|part| part.is_empty() || *part == "..") {
        return None;
    }
    let package = parts[..name_len].join("/");
    let containing_file = combine_paths(package_directory, &["package.json"]);
    let resolved = resolver
        .resolve_package_directory(&package, &containing_file, RESOLUTION_MODE_NONE, None)
        .filter(|resolved| !resolved.resolved_file_name.is_empty())?;
    let path = combine_paths(&resolved.resolved_file_name, &parts[name_len..]);
    let fs = host.fs();
    if fs.file_exists(&path) {
        return Some(path);
    }
    let exe = format!("{path}.exe");
    (platform == "win32" && fs.file_exists(&exe)).then_some(exe)
}

#[cfg(test)]
mod platform_exec_tests {
    use super::*;
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
    fn platform_exec_names_the_installed_native_binary() {
        let root =
            std::env::temp_dir().join(format!("goport-platform-exec-{}", std::process::id()));
        let root = std::path::PathBuf::from(os_fs_realpath_dir(&root));
        let (platform, arch) = node_platform();
        let modules = root.join("node_modules");
        write(
            &modules.join("@x/m/package.json"),
            r#"{"name":"@x/m","version":"1.0.0","typescript":{"contentMapper":{"exec":["./bin/m"],"platformExec":"@x/m-${platform}-${arch}/lib/m"}}}"#,
        );
        write(
            &modules.join("missing/package.json"),
            r#"{"name":"missing","typescript":{"contentMapper":{"exec":["./bin/m"],"platformExec":"missing-${platform}-${arch}/lib/m"}}}"#,
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
