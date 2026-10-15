//! End to end through the language service: a `.sts` file mapped by the
//! real `tsc-ext` process. Hover on a macro invocation names the macro and
//! shows its output, `custom/textDocument/showExpansion` returns the same,
//! and a diagnostic inside macro output points at the invocation with an
//! "Expanded from" related location.

use std::rc::Rc;

use ts_goport::contentmapper::SpawnerFunc;
use ts_goport::frontend::bundled;
use ts_goport::frontend::vfs::osvfs_fs;
use ts_goport::gostd::context;
use ts_goport::lsp::lsproto;
use ts_goport::project::{self, Session, SessionInit, SessionOptions};

const SOURCE: &str = "export syntax unless:stmt {\n  rule { unless ($c:expr) $b:stmt } => { if (!($c)) $b }\n}\nexport function step(n: number): number {\n  unless (n < 0) { return n + 1; }\n  return 0;\n}\nexport function bad(n: number): string {\n  unless (n < 0) { return n; }\n  return \"\";\n}\n";

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
    let dir = std::env::temp_dir().join(format!("ts_ext_lsp_{name}_{}", std::process::id()));
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

// One test: a session publishes process-wide file ids, so two sessions on
// parallel test threads would race.
#[test]
fn hover_show_expansion_and_diagnostics_over_macro_output() {
    let p = project("lsp");
    let ls = p
        .session
        .get_language_service(&ctx(), &p.uri())
        .unwrap_or_else(|err| panic!("{}", err.error()));
    // On `unless` in `step` (line 4, character 3).
    let position = lsproto::Position {
        line: 4,
        character: 3,
    };
    let expansion = ts_goport::ext_ls::expansion_at(&ls, &p.uri(), position)
        .expect("an expansion at the invocation");
    assert!(expansion.label.contains("unless"), "{}", expansion.label);
    assert!(
        expansion.text.contains("if (!(n < 0))"),
        "{}",
        expansion.text
    );
    assert_eq!(expansion.range.start.line, 4);
    let hover = ts_goport::ext_ls::provide_hover(
        &ls,
        &ctx(),
        &lsproto::HoverParams {
            text_document: lsproto::TextDocumentIdentifier { uri: p.uri() },
            position,
            ..Default::default()
        },
    )
    .unwrap_or_else(|err| panic!("{}", err.error()));
    let text = hover
        .hover
        .as_ref()
        .and_then(|hover| hover.contents.markup_content.as_ref())
        .map(|markup| markup.value.clone())
        .unwrap_or_default();
    assert!(text.contains("Expanded from"), "{text}");
    assert!(text.contains("if (!(n < 0))"), "{text}");
    // Outside every invocation there is none.
    let outside = lsproto::Position {
        line: 3,
        character: 17,
    };
    assert!(ts_goport::ext_ls::expansion_at(&ls, &p.uri(), outside).is_none());
    let report = ls
        .provide_diagnostics(&ctx(), &p.uri())
        .unwrap_or_else(|err| panic!("{}", err.error()));
    let rendered = format!("{report:?}");
    // `return n` in `bad` returns a number where a string is expected.
    assert!(rendered.contains("2322"), "{rendered}");
    assert!(rendered.contains("Expanded from"), "{rendered}");
}
