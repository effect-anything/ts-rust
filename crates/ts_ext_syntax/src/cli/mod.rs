//! The `tsc-ext` command line, apart from the content mapper itself
//! (`src/bin/tsc_ext.rs` serves that on stdin/stdout).
//!
//! Exit codes: 0 ok (warnings allowed), 1 error diagnostics, 2 usage,
//! 3 I/O or config. Output code goes to stdout, diagnostics and traces to
//! stderr.

pub mod args;
pub mod docs;
pub mod expand;
pub mod render;

use args::{Flag, Takes, flag};

pub const EXIT_OK: u8 = 0;
pub const EXIT_ERRORS: u8 = 1;
pub const EXIT_USAGE: u8 = 2;
pub const EXIT_CONFIG: u8 = 3;

const EXPAND_FLAGS: &[Flag] = &[
    flag("stage", Takes::Value),
    flag("only", Takes::Value),
    flag("tsconfig", Takes::Value),
    flag("map", Takes::OptionalValue),
    flag("json", Takes::Nothing),
    flag("diff", Takes::Nothing),
    flag("trace", Takes::Nothing),
    flag("format", Takes::Value),
    flag("color", Takes::Value),
];

const EVAL_FLAGS: &[Flag] = &[
    flag("stage", Takes::Value),
    flag("only", Takes::Value),
    flag("tsconfig", Takes::Value),
    flag("map", Takes::OptionalValue),
    flag("json", Takes::Nothing),
    flag("diff", Takes::Nothing),
    flag("trace", Takes::Nothing),
    flag("format", Takes::Value),
    flag("color", Takes::Value),
    flag("jsx", Takes::Nothing),
    flag("lang", Takes::Value),
    flag("as", Takes::Value),
];

const DOC_FLAGS: &[Flag] = &[
    flag("raw", Takes::Nothing),
    flag("list", Takes::Nothing),
    flag("color", Takes::Value),
];

pub const HELP: &str = "\
tsc-ext — the @effect-x/tsc content mapper, and tools to debug what it does

USAGE
  tsc-ext <command> [options]

COMMANDS
  expand <file>...     print what a .sts/.stsx/.ts file becomes, with its diagnostics
  eval [code|-]        the same for code given as an argument or on stdin
  explain <code>       what a diagnostic code (SWR4001, ...) means and how to fix it
  cheatsheet [topic]   syntax quick reference, options and diagnostics docs
  help [command]       this text, or a command's help (same as <command> --help)

  With no command, tsc-ext is the content mapper: it speaks the content-mapper
  protocol on stdin/stdout. tsc starts it for a tsconfig \"contentMappers\" entry
  of @effect-x/tsc when run with --runExternalCode; you do not run it by hand.

STAGES (expand, eval)
  tsc      what tsc type-checks: Sweetener expands .sts/.stsx; other files are
           read as written (default)
  bundle   what the bundler plugin emits: Sweetener, then the tsover operator
           rewrite and the TypeGPU wrap (__TYPEGPU_META__)

EXIT CODES
  0 ok (warnings allowed)   1 error diagnostics   2 usage   3 I/O or config error

EXAMPLES
  tsc-ext expand src/step.sts
  tsc-ext expand src/step.sts --trace
  tsc-ext expand src/gpu.ts --stage bundle --diff
  echo 'unless (done) { work(); }' | tsc-ext eval --as src/scratch.sts
  tsc-ext explain SWR4001
  tsc-ext cheatsheet macros
";

pub const EXPAND_HELP: &str = "\
tsc-ext expand — print what a file becomes at a stage

USAGE
  tsc-ext expand <file>... [options]

  Code goes to stdout, diagnostics (with a code frame) and the trace to stderr.
  Several files print one after another, each after a `// ==> file <==` line.

STAGES
  --stage tsc       what tsc type-checks (default). Sweetener expands .sts/.stsx
                    (check phase); a .ts/.tsx file is printed as written, since tsc
                    checks \"use gpu\" functions and overloaded operators natively.
  --stage bundle    what the bundler plugin emits: Sweetener (build phase), then
                    tsover rewrites operators in \"use gpu\" functions to
                    __tsover_* calls, then TypeGPU wraps them with __TYPEGPU_META__.
                    TypeGPU options come from the tsconfig.

OPTIONS
  --only <pass>     run one pass: sweetener, typegpu or tsover (the last two need
                    --stage bundle; on a .sts file they run on the Sweetener output)
  --tsconfig <path> tsconfig whose @effect-x/tsc options apply (default: the
                    nearest tsconfig.json above the file)
  --map             append the source map inline (//# sourceMappingURL=data:...)
  --map=<file>      write the source map to <file> (one input only)
  --json            print one JSON object (an array for several files) on stdout:
                    {file, stage, code, diagnostics, map?, trace?, notes?}
  --diff            print a unified diff from the input to the output instead
  --trace           print the macro trace to stderr: every rule tried at every use,
                    where and why it stopped, and each replacement
  --format tsc      diagnostics as tsc prints them, file(line,col), without frames
  --color <when>    auto (default; off when NO_COLOR is set), always, never
  -h, --help        this text

EXIT CODES
  0 ok (warnings allowed)   1 error diagnostics   2 usage   3 I/O or config error

EXAMPLES
  tsc-ext expand src/step.sts                     # the TypeScript tsc checks
  tsc-ext expand src/step.sts --trace             # why a macro did (not) match
  tsc-ext expand src/step.sts --diff              # what changed, as a diff
  tsc-ext expand src/gpu.ts --stage bundle --only typegpu
  tsc-ext expand src/*.sts --json > out.json
  tsc-ext expand src/step.sts --map=step.ts.map > step.ts
";

pub const EVAL_HELP: &str = "\
tsc-ext eval — expand code given as an argument or on stdin

USAGE
  tsc-ext eval [code|-] [options]

  With no code (or -), the code is read from stdin. Output, stages and exit codes
  are those of `tsc-ext expand` (see `tsc-ext expand --help`).

FILE KIND
  The code is expanded as a file named <eval>.sts in the current directory, so
  Sweetener runs. Choose another kind with one of:
  --jsx             a .stsx file (Sweetener with JSX)
  --lang ts|tsx     a plain .ts/.tsx file: no Sweetener; use --stage bundle to see
                    the tsover and TypeGPU rewrites
  --as <path>       the file it stands for, e.g. src/scratch.stsx. Its extension
                    picks the kind; relative `for syntax` imports and the tsconfig
                    lookup start from its directory. The file need not exist.

OPTIONS
  --stage tsc|bundle, --only <pass>, --tsconfig <path>, --map[=<file>], --json,
  --diff, --trace, --format tsc, --color <when>: as for expand.
  -h, --help        this text

EXIT CODES
  0 ok (warnings allowed)   1 error diagnostics   2 usage   3 I/O or config error

EXAMPLES
  tsc-ext eval 'unless (ready) { wait(); }' --as src/x.sts
  echo 'const v = a + b;' | tsc-ext eval --lang ts --stage bundle
  tsc-ext eval --jsx '<When cond={ok}>yes</When>' --trace
  pbpaste | tsc-ext eval --as src/macros-test.sts --diff
";

pub const EXPLAIN_HELP: &str = "\
tsc-ext explain — what a Sweetener diagnostic code means and how to fix it

USAGE
  tsc-ext explain <code>      SWR4001, swr4001 or 4001
  tsc-ext explain --list      every code with its title

  Prints the title, an explanation, a small file that triggers the code, and the
  fix. Every diagnostic tsc-ext prints ends with the explain line for its code.

OPTIONS
  --list            list every code
  --raw             print Markdown instead of formatted text
  --color <when>    auto (default), always, never
  -h, --help        this text

EXIT CODES
  0 found   2 unknown code or usage

EXAMPLES
  tsc-ext explain SWR4001
  tsc-ext explain 2013 --raw >> notes.md
";

pub const CHEATSHEET_HELP: &str = "\
tsc-ext cheatsheet — syntax quick reference and docs, built into the binary

USAGE
  tsc-ext cheatsheet [topic]   no topic: the whole cheatsheet
  tsc-ext cheatsheet --list    the topics

TOPICS
  Run `tsc-ext cheatsheet --list`. They include the Sweetener syntax (macros,
  `for syntax` imports, patterns, templates, built-in operations), \"use gpu\",
  operator overloading and tsover, plus `options` (the tsconfig reference) and
  `diagnostics` (every SWR code).

OPTIONS
  --list            list the topics
  --raw             print Markdown instead of formatted text
  --color <when>    auto (default), always, never
  -h, --help        this text

EXIT CODES
  0 ok   2 unknown topic or usage

EXAMPLES
  tsc-ext cheatsheet
  tsc-ext cheatsheet templates
  tsc-ext cheatsheet options --raw | less
";

fn help_for(command: &str) -> Option<&'static str> {
    match command {
        "expand" => Some(EXPAND_HELP),
        "eval" => Some(EVAL_HELP),
        "explain" => Some(EXPLAIN_HELP),
        "cheatsheet" => Some(CHEATSHEET_HELP),
        "help" => Some(HELP),
        _ => None,
    }
}

/// A usage error: the message, then where to read more. Exit 2.
pub fn usage_error(command: &str, message: &str) -> u8 {
    eprintln!("tsc-ext {command}: {message}\nRun `tsc-ext {command} --help` for usage.");
    EXIT_USAGE
}

/// Run a command (`args` without the program name). The caller has handled
/// the content-mapper case (no arguments, or `mapper`).
#[must_use]
pub fn run(args: &[String]) -> u8 {
    let Some(command) = args.first().map(String::as_str) else {
        print!("{HELP}");
        return EXIT_OK;
    };
    let rest = &args[1..];
    match command {
        "-h" | "--help" => {
            print!("{HELP}");
            EXIT_OK
        }
        "help" => match rest.first() {
            None => {
                print!("{HELP}");
                EXIT_OK
            }
            Some(topic) => match help_for(topic) {
                Some(text) => {
                    print!("{text}");
                    EXIT_OK
                }
                None => usage_error(
                    "help",
                    &format!("no command {topic}; commands are expand, eval, explain, cheatsheet"),
                ),
            },
        },
        "expand" => with(command, rest, EXPAND_FLAGS, expand::expand),
        "eval" => with(command, rest, EVAL_FLAGS, expand::eval),
        "explain" => with(command, rest, DOC_FLAGS, explain),
        "cheatsheet" => with(command, rest, DOC_FLAGS, cheatsheet),
        other => {
            eprintln!("tsc-ext: unknown command {other}\n\n{HELP}");
            EXIT_USAGE
        }
    }
}

fn with(command: &str, rest: &[String], flags: &[Flag], body: fn(&args::Parsed) -> u8) -> u8 {
    match args::parse(rest, flags) {
        Ok(parsed) if parsed.help => {
            print!("{}", help_for(command).unwrap_or(HELP));
            EXIT_OK
        }
        Ok(parsed) => body(&parsed),
        Err(message) => usage_error(command, &message),
    }
}

fn print_doc(parsed: &args::Parsed, markdown: &str) -> u8 {
    let style = match render::ColorChoice::parse(parsed.value("color")) {
        Ok(choice) => choice.stdout(),
        Err(message) => return usage_error("cheatsheet", &message),
    };
    if parsed.has("raw") {
        print!("{markdown}");
    } else {
        print!("{}", docs::render(markdown, style));
    }
    EXIT_OK
}

fn explain(parsed: &args::Parsed) -> u8 {
    if parsed.has("list") {
        print!("{}", docs::list());
        return EXIT_OK;
    }
    let [code] = parsed.positionals.as_slice() else {
        return usage_error(
            "explain",
            "name one code: tsc-ext explain SWR4001 (or --list)",
        );
    };
    match docs::explain(code) {
        Ok(markdown) => print_doc(parsed, &markdown),
        Err(message) => usage_error("explain", &message),
    }
}

fn cheatsheet(parsed: &args::Parsed) -> u8 {
    if parsed.has("list") {
        for (name, title) in docs::topics() {
            println!("{name:<12} {title}");
        }
        return EXIT_OK;
    }
    if parsed.positionals.len() > 1 {
        return usage_error("cheatsheet", "name at most one topic");
    }
    match docs::topic(parsed.positionals.first().map(String::as_str)) {
        Some(markdown) => print_doc(parsed, &markdown),
        None => {
            let names: Vec<String> = docs::topics().into_iter().map(|(name, _)| name).collect();
            usage_error(
                "cheatsheet",
                &format!(
                    "no topic {}; topics are {}",
                    parsed.positionals[0],
                    names.join(", ")
                ),
            )
        }
    }
}
