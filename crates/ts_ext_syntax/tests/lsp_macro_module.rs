//! The language server and a `for syntax` macro module: after the module
//! changes on disk (a watched-file event), the file that imports it is
//! expanded and checked again, though its own text did not change. The
//! content-mapped parse cache keys a file by its dependencies too
//! (`ext::mapper_deps`), and the change marks the importer changed.
//!
//! The same session then answers the content-mapper API methods
//! (`getMappedFileInfo`, `getOriginalPositions`, ...), through the
//! dispatch of `tsc --api`, in UTF-16 positions.

use std::rc::Rc;

use ts_goport::contentmapper::SpawnerFunc;
use ts_goport::frontend::bundled;
use ts_goport::frontend::vfs::osvfs_fs;
use ts_goport::gostd::context;
use ts_goport::lsp::lsproto;
use ts_goport::project::{self, Session, SessionInit, SessionOptions};

const PAIR: &str = "export syntax twice:expr {\n  rule { twice($x:expr) } => { [$x, $x] }\n}\n";
const TRIPLE: &str = "export syntax twice:expr {\n  rule { twice($x:expr) } => { [$x, $x, $x] }\n}\n";
const SOURCE: &str = "import { twice } from \"./macros/twice.sts\" for syntax;\nexport const caf\u{e9}: [number, number] = twice(1);\n";

struct Project {
    dir: std::path::PathBuf,
    session: Rc<Session>,
}

impl Drop for Project {
    fn drop(&mut self) {
        self.session.close();
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

impl Project {
    fn uri(&self) -> lsproto::DocumentUri {
        lsproto::DocumentUri(format!("file://{}/app.sts", self.dir.display()))
    }
}

fn project(name: &str) -> Project {
    let dir = std::env::temp_dir().join(format!("ts_ext_lsp_deps_{name}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("node_modules/mapper")).unwrap();
    let dir = dir.canonicalize().unwrap();
    std::fs::write(
        dir.join("tsconfig.json"),
        r#"{ "compilerOptions": { "target": "es2020", "module": "esnext", "moduleResolution": "bundler", "strict": true },
  "contentMappers": [ { "package": "mapper", "extensions": [".sts"] } ] }"#,
    )
    .unwrap();
    std::fs::write(
        dir.join("node_modules/mapper/package.json"),
        format!(
            r#"{{ "name": "mapper", "version": "1.0.0", "typescript": {{ "contentMapper": {{ "exec": [{:?}] }} }} }}"#,
            env!("CARGO_BIN_EXE_tsc-ext")
        ),
    )
    .unwrap();
    std::fs::create_dir_all(dir.join("macros")).unwrap();
    std::fs::write(dir.join("macros/twice.sts"), PAIR).unwrap();
    std::fs::write(dir.join("app.sts"), SOURCE).unwrap();
    let session = project::new_session(&SessionInit {
        background_ctx: context::background(),
        options: Rc::new(SessionOptions {
            current_directory: dir.display().to_string(),
            default_library_path: bundled::lib_path(),
            typings_location: String::new(),
            position_encoding: lsproto::PositionEncodingKind::UTF16,
            watch_enabled: false,
            logging_enabled: false,
            telemetry_enabled: false,
            push_diagnostics_enabled: false,
            run_external_code: true,
            debounce_delay: std::time::Duration::ZERO,
            checker_pool_options: Default::default(),
        }),
        fs: bundled::wrap_fs(osvfs_fs()),
        client: None,
        logger: None,
        npm_executor: None,
        spawner: Some(Rc::new(SpawnerFunc(Box::new(|command, dir, stderr| {
            ts_goport::execute::tsc::spawn_process(command, dir, stderr)
        })))),
        content_mapper_logger: None,
        parse_cache: None,
        content_mapped_parse_cache: None,
    });
    let project = Project { dir, session };
    project.session.did_open_file(
        &context::background(),
        &project.uri(),
        1,
        SOURCE,
        &lsproto::LanguageKind("sts".into()),
    );
    project
}

/// A request context whose client accepts related information.
fn ctx() -> context::Context {
    let mut caps = lsproto::ResolvedClientCapabilities::default();
    caps.text_document.diagnostic.related_information = true;
    let ctx = lsproto::with_client_capabilities(&context::background(), std::sync::Arc::new(caps));
    ts_goport::frontend::core_context::with_request_id(&ctx, "0")
}

fn diagnostics(p: &Project) -> String {
    let ls = p
        .session
        .get_language_service(&ctx(), &p.uri())
        .unwrap_or_else(|err| panic!("{}", err.error()));
    let report = ls
        .provide_diagnostics(&ctx(), &p.uri())
        .unwrap_or_else(|err| panic!("{}", err.error()));
    format!("{report:?}")
}

fn changed(p: &Project, text: &str) {
    std::fs::write(p.dir.join("macros/twice.sts"), text).unwrap();
    let uri = lsproto::DocumentUri(format!("file://{}/macros/twice.sts", p.dir.display()));
    p.session.did_change_watched_files(
        &context::background(),
        &[Some(lsproto::FileEvent {
            uri,
            type_: lsproto::FileChangeType::CHANGED,
        })],
    );
}

#[test]
fn an_edited_macro_module_rechecks_the_open_importer() {
    let p = project("edit");
    let before = diagnostics(&p);
    assert!(!before.contains("2322"), "{before}");
    changed(&p, TRIPLE);
    let after = diagnostics(&p);
    assert!(after.contains("2322"), "after the edit:\n{after}");
    changed(&p, PAIR);
    let restored = diagnostics(&p);
    assert!(!restored.contains("2322"), "after the revert:\n{restored}");
    api_methods(&p);
}

fn call(session: &ts_goport::api::Session, method: &str, params: serde_json::Value) -> serde_json::Value {
    use ts_goport::frontend::json::MarshalerTo;
    use ts_goport::ipc::Handler;
    let result = session
        .handle_request(
            &context::background(),
            method,
            ts_goport::frontend::json_ext::JsonValue(params.to_string().into_bytes()),
        )
        .unwrap_or_else(|err| panic!("{method}: {}", err.error()));
    let mut out = String::new();
    match result {
        Some(value) => value.marshal_json_to(&mut out).unwrap(),
        None => out.push_str("null"),
    }
    serde_json::from_str(&out).unwrap_or_else(|err| panic!("{method}: {err}: {out}"))
}

fn api_methods(p: &Project) {
    let session = ts_goport::api::new_lsp_session(p.session.clone(), None);
    let app = format!("{}/app.sts", p.dir.display());
    let response = session
        .handle_get_current_language_server_snapshot(
            &context::background(),
            &ts_goport::api::GetCurrentLanguageServerSnapshotParams {
                base_snapshot: Default::default(),
                changes: Some(ts_goport::api::LanguageServerSnapshotChanges {
                    snapshot_request_changes_params: ts_goport::api::SnapshotRequestChangesParams {
                        open_files: Some(vec![ts_goport::api::DocumentIdentifier {
                            file_name: app.clone(),
                            ..Default::default()
                        }]),
                        ..Default::default()
                    },
                }),
            },
        )
        .unwrap_or_else(|err| panic!("{}", err.error()));
    let base = serde_json::json!({
        "snapshot": response.snapshot.0,
        "project": response.projects[0].id.0,
    });
    let with = |extra: serde_json::Value| {
        let mut params = base.clone();
        params.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
        params
    };

    let info = call(&session, "getMappedFileInfo", with(serde_json::json!({ "file": app })));
    assert_eq!(info["mapped"], true, "{info}");
    let macros = format!("{}/macros/twice.sts", p.dir.display());
    assert_eq!(info["dependencies"], serde_json::json!([macros]), "{info}");
    let import = &info["syntaxImports"][0];
    assert_eq!(import["specifier"], "./macros/twice.sts", "{info}");
    assert_eq!(import["resolved"], macros.as_str(), "{info}");
    assert_eq!(import["bindings"][0]["local"], "twice", "{info}");
    assert_eq!(import["originalStart"], 0, "{info}");
    let expansion = &info["expansions"][0];
    assert!(expansion["label"].as_str().unwrap().contains("twice"), "{info}");
    let kinds: Vec<&str> = info["regions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["kind"].as_str().unwrap())
        .collect();
    assert!(kinds.contains(&"verbatim") && kinds.contains(&"generated"), "{info}");

    // UTF-16: `twice(1)` starts after `café` (one UTF-16 unit, two bytes).
    let line1 = SOURCE.find('\n').unwrap() + 1;
    let call_at = SOURCE.find("twice(1)").unwrap();
    let utf16 = |byte: usize| SOURCE[..byte].encode_utf16().count();
    assert_eq!(expansion["originalStart"], utf16(call_at), "{info}");
    // `café` is user text: it maps exactly both ways.
    let cafe = utf16(line1 + "export const ".len());
    let virtual_cafe = call(&session, "getVirtualPositions", with(serde_json::json!({ "file": app, "positions": [cafe] })));
    let v = virtual_cafe[0][0]["position"].as_i64().unwrap();
    assert_eq!(virtual_cafe[0][0]["fidelity"], "exact", "{virtual_cafe}");
    let back = call(&session, "getOriginalPositions", with(serde_json::json!({ "file": app, "positions": [v] })));
    assert_eq!(back[0]["position"], cafe, "{back}");
    assert_eq!(back[0]["fidelity"], "exact", "{back}");

    // Batch: every mapped file, and an aligned list with a missing file.
    let all = call(&session, "getMappedFilesInfo", base.clone());
    assert!(all.as_array().unwrap().iter().any(|f| f["fileName"] == app.as_str()), "{all}");
    let some = call(&session, "getMappedFilesInfo", with(serde_json::json!({ "files": [app, "/nope.ts"] })));
    assert_eq!(some[1], serde_json::Value::Null, "{some}");
    assert_eq!(call(&session, "getMappedFileInfo", with(serde_json::json!({ "file": "/nope.ts" }))), serde_json::Value::Null);
    session.close();
}
