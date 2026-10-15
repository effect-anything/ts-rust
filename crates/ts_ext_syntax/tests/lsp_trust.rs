//! The language server and the content-mapper trust gate.
//!
//! A `contentMappers` entry runs code from `node_modules`, so the server
//! honors it only when the user enabled `runExternalCode`. Without it the
//! config reports TS18068, the mapper is dropped, `.sts` is no extension
//! the project knows, and the file belongs to no project: every request on
//! it fails with "no project found". This is what an editor that never sends
//! `initializationOptions.runExternalCode` shows. `tsc --lsp --stdio
//! --runExternalCode` (see `ext::lsp_trust`) is the default for those
//! editors; this test pins both outcomes at the session.

use std::rc::Rc;

use ts_goport::contentmapper::SpawnerFunc;
use ts_goport::frontend::bundled;
use ts_goport::frontend::vfs::osvfs_fs;
use ts_goport::gostd::context;
use ts_goport::lsp::lsproto;
use ts_goport::project::{self, Session, SessionInit, SessionOptions};

const PAIR: &str = "export syntax twice:expr {\n  rule { twice($x:expr) } => { [$x, $x] }\n}\n";
const SOURCE: &str = "import { twice } from \"./macros/twice.sts\" for syntax;\nexport const pair: [number, number] = twice(1);\n";

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

fn project(name: &str, trusted: bool) -> Project {
    let dir = std::env::temp_dir().join(format!("ts_ext_lsp_trust_{name}_{}", std::process::id()));
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
            run_external_code: trusted,
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


fn ctx() -> context::Context {
    let mut caps = lsproto::ResolvedClientCapabilities::default();
    caps.text_document.diagnostic.related_information = true;
    let ctx = lsproto::with_client_capabilities(&context::background(), std::sync::Arc::new(caps));
    ts_goport::frontend::core_context::with_request_id(&ctx, "0")
}

fn language_service_error(p: &Project) -> Option<String> {
    p.session.get_language_service(&ctx(), &p.uri()).err().map(|e| e.error())
}

#[test]
fn an_untrusted_workspace_has_no_project_for_a_mapped_file() {
    let p = project("off", false);
    let err = language_service_error(&p).expect("no project without runExternalCode");
    assert!(err.contains("no project found"), "{err}");
}

#[test]
fn a_trusted_workspace_opens_the_mapped_file_and_reads_no_config_error() {
    let p = project("on", true);
    assert_eq!(language_service_error(&p), None);
    let ls = p.session.get_language_service(&ctx(), &p.uri()).unwrap();
    let report = format!("{:?}", ls.provide_diagnostics(&ctx(), &p.uri()).unwrap());
    assert!(!report.contains("18068") && !report.contains("2322"), "{report}");
}
