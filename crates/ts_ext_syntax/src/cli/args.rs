//! Command-line parsing, hand-rolled as the rest of this workspace does it
//! (`ts_goport`'s tsc reads its own options; no argument-parsing crate is
//! a dependency anywhere). Long flags only, plus `-h`: `--flag`,
//! `--flag value`, `--flag=value`, and `--` to end the flags.

/// Whether a flag takes a value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Takes {
    /// `--json`.
    Nothing,
    /// `--stage tsc` or `--stage=tsc`.
    Value,
    /// `--map` or `--map=file` (only the `=` form gives the value).
    OptionalValue,
}

/// One flag a command accepts.
#[derive(Clone, Copy, Debug)]
pub struct Flag {
    pub name: &'static str,
    pub takes: Takes,
}

#[must_use]
pub const fn flag(name: &'static str, takes: Takes) -> Flag {
    Flag { name, takes }
}

/// Parsed arguments: flags in order and positionals.
#[derive(Debug, Default)]
pub struct Parsed {
    pub flags: Vec<(&'static str, Option<String>)>,
    pub positionals: Vec<String>,
    pub help: bool,
}

impl Parsed {
    #[must_use]
    pub fn has(&self, name: &str) -> bool {
        self.flags.iter().any(|(flag, _)| *flag == name)
    }

    /// The last value of `--name` (a later flag overrides an earlier one).
    #[must_use]
    pub fn value(&self, name: &str) -> Option<&str> {
        self.flags
            .iter()
            .rev()
            .find(|(flag, _)| *flag == name)
            .and_then(|(_, value)| value.as_deref())
    }
}

/// Parse `args` against `flags`. `Err` is the usage message.
///
/// # Errors
/// An unknown flag, a missing value, or a value given to a flag that takes none.
pub fn parse(args: &[String], flags: &[Flag]) -> Result<Parsed, String> {
    let mut parsed = Parsed::default();
    let mut rest = args.iter();
    let mut only_positionals = false;
    while let Some(arg) = rest.next() {
        if only_positionals || arg == "-" || !arg.starts_with('-') {
            parsed.positionals.push(arg.clone());
            continue;
        }
        if arg == "--" {
            only_positionals = true;
            continue;
        }
        if arg == "-h" || arg == "--help" {
            parsed.help = true;
            continue;
        }
        let Some(body) = arg.strip_prefix("--") else {
            return Err(format!("unknown option {arg} (options are spelled --name)"));
        };
        let (name, inline) = match body.split_once('=') {
            Some((name, value)) => (name, Some(value.to_owned())),
            None => (body, None),
        };
        let Some(spec) = flags.iter().find(|spec| spec.name == name) else {
            let known: Vec<String> = flags
                .iter()
                .map(|spec| format!("--{}", spec.name))
                .collect();
            return Err(format!(
                "unknown option --{name}; this command takes {}",
                known.join(", ")
            ));
        };
        let value = match spec.takes {
            Takes::Nothing => {
                if inline.is_some() {
                    return Err(format!("--{name} takes no value"));
                }
                None
            }
            Takes::OptionalValue => inline,
            Takes::Value => match inline {
                Some(value) => Some(value),
                None => match rest.next() {
                    Some(value) => Some(value.clone()),
                    None => return Err(format!("--{name} needs a value")),
                },
            },
        };
        parsed.flags.push((spec.name, value));
    }
    Ok(parsed)
}

#[cfg(test)]
mod tests {
    use super::*;

    const FLAGS: &[Flag] = &[
        flag("json", Takes::Nothing),
        flag("stage", Takes::Value),
        flag("map", Takes::OptionalValue),
    ];

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|arg| (*arg).to_owned()).collect()
    }

    #[test]
    fn forms() {
        let parsed = parse(
            &args(&[
                "a.sts", "--stage", "bundle", "--map", "--json", "-", "--", "--x",
            ]),
            FLAGS,
        )
        .unwrap();
        assert_eq!(parsed.value("stage"), Some("bundle"));
        assert!(parsed.has("map") && parsed.value("map").is_none() && parsed.has("json"));
        assert_eq!(parsed.positionals, ["a.sts", "-", "--x"]);
        let parsed = parse(&args(&["--stage=tsc", "--map=out.map"]), FLAGS).unwrap();
        assert_eq!(
            (parsed.value("stage"), parsed.value("map")),
            (Some("tsc"), Some("out.map"))
        );
        assert!(
            parse(&args(&["--nope"]), FLAGS)
                .unwrap_err()
                .contains("unknown option --nope")
        );
        assert!(
            parse(&args(&["--stage"]), FLAGS)
                .unwrap_err()
                .contains("needs a value")
        );
        assert!(
            parse(&args(&["--json=1"]), FLAGS)
                .unwrap_err()
                .contains("takes no value")
        );
        assert!(parse(&args(&["-x"]), FLAGS).is_err());
        assert!(parse(&args(&["-h"]), FLAGS).unwrap().help);
    }
}
