//! Command line: a hand-written grammar over `args_os()` (design D2: no clap).

use std::ffi::{OsStr, OsString};
use std::io::Write;
use std::path::PathBuf;
use std::process::ExitCode;

/// The version printed by `--version`; release tags must equal it (CI checks).
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Printed by `--help` and `<command> --help` to stdout, and on usage errors to stderr.
pub const USAGE: &str = "\
Usage:
  kohaku serve              Run the web server on port 8080
  kohaku healthcheck        Check that the local server answers (for container health checks)
  kohaku backup <file>      Write a backup of the database to a new file
  kohaku backup -           Write a backup of the database to standard output
  kohaku restore <file>     Replace the database with a backup (server stopped)
  kohaku restore -          Replace the database with a backup read from standard input
  kohaku restore --list     List the files in /data/backups
  kohaku --version          Print the version
  kohaku --help             Print this help
  kohaku <command> --help   Print this help

Settings come from environment variables only; see the README.
";

/// Exit status of a usage error; every other failure exits 1.
const EXIT_USAGE: u8 = 2;

/// What one invocation asks for.
#[derive(Debug, PartialEq, Eq)]
pub enum Invocation {
    Version,
    Help,
    Run(Command),
}

#[derive(Debug, PartialEq, Eq)]
pub enum Command {
    Serve,
    Healthcheck,
    Backup(BackupTarget),
    Restore(RestoreSource),
    RestoreList,
}

#[derive(Debug, PartialEq, Eq)]
pub enum BackupTarget {
    File(PathBuf),
    Stdout,
}

#[derive(Debug, PartialEq, Eq)]
pub enum RestoreSource {
    File(PathBuf),
    Stdin,
}

/// Any invocation outside the grammar. Carries no detail: the answer is always the
/// usage text, never an echo of the arguments.
#[derive(Debug, PartialEq, Eq)]
pub struct UsageError;

/// Parses the arguments after the program name. Names and flags match exactly, case
/// included; a path is any argument that is not empty and does not start with `-`.
pub fn parse<I>(args: I) -> Result<Invocation, UsageError>
where
    I: IntoIterator<Item = OsString>,
{
    let args: Vec<OsString> = args.into_iter().collect();
    let args: Vec<&OsStr> = args.iter().map(OsString::as_os_str).collect();
    let command_help = |rest: &[&OsStr]| rest == [OsStr::new("--help")];

    match args.as_slice() {
        [flag] if *flag == "--version" => Ok(Invocation::Version),
        [flag] if *flag == "--help" => Ok(Invocation::Help),
        [name, rest @ ..] if *name == "serve" => match rest {
            [] => Ok(Invocation::Run(Command::Serve)),
            _ if command_help(rest) => Ok(Invocation::Help),
            _ => Err(UsageError),
        },
        [name, rest @ ..] if *name == "healthcheck" => match rest {
            [] => Ok(Invocation::Run(Command::Healthcheck)),
            _ if command_help(rest) => Ok(Invocation::Help),
            _ => Err(UsageError),
        },
        [name, rest @ ..] if *name == "backup" => match rest {
            _ if command_help(rest) => Ok(Invocation::Help),
            [arg] if *arg == "-" => Ok(Invocation::Run(Command::Backup(BackupTarget::Stdout))),
            [arg] if is_path(arg) => Ok(Invocation::Run(Command::Backup(BackupTarget::File(
                PathBuf::from(arg),
            )))),
            _ => Err(UsageError),
        },
        [name, rest @ ..] if *name == "restore" => match rest {
            _ if command_help(rest) => Ok(Invocation::Help),
            [arg] if *arg == "--list" => Ok(Invocation::Run(Command::RestoreList)),
            [arg] if *arg == "-" => Ok(Invocation::Run(Command::Restore(RestoreSource::Stdin))),
            [arg] if is_path(arg) => Ok(Invocation::Run(Command::Restore(RestoreSource::File(
                PathBuf::from(arg),
            )))),
            _ => Err(UsageError),
        },
        _ => Err(UsageError),
    }
}

fn is_path(arg: &OsStr) -> bool {
    !arg.is_empty() && !arg.as_encoded_bytes().starts_with(b"-")
}

/// Entry point of the `kohaku` binary.
pub fn main() -> ExitCode {
    match parse(std::env::args_os().skip(1)) {
        Ok(Invocation::Version) => write_stdout(&format!("kohaku {VERSION}\n")),
        Ok(Invocation::Help) => write_stdout(USAGE),
        Ok(Invocation::Run(command)) => run(command),
        Err(UsageError) => {
            // Nothing sensible remains to do if stderr itself is gone.
            let _ = std::io::stderr().write_all(USAGE.as_bytes());
            ExitCode::from(EXIT_USAGE)
        }
    }
}

fn run(command: Command) -> ExitCode {
    match command {
        Command::Serve
        | Command::Healthcheck
        | Command::Backup(_)
        | Command::Restore(_)
        | Command::RestoreList => {
            let _ = writeln!(std::io::stderr(), "kohaku: this command is not built yet");
            ExitCode::FAILURE
        }
    }
}

fn write_stdout(text: &str) -> ExitCode {
    let mut stdout = std::io::stdout().lock();
    match stdout
        .write_all(text.as_bytes())
        .and_then(|()| stdout.flush())
    {
        Ok(()) => ExitCode::SUCCESS,
        Err(_) => ExitCode::FAILURE,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(args: &[&str]) -> Result<Invocation, UsageError> {
        parse(args.iter().map(OsString::from))
    }

    #[test]
    fn accepts_every_invocation() {
        use Invocation::*;
        assert_eq!(p(&["--version"]), Ok(Version));
        assert_eq!(p(&["--help"]), Ok(Help));
        assert_eq!(p(&["serve"]), Ok(Run(Command::Serve)));
        assert_eq!(p(&["healthcheck"]), Ok(Run(Command::Healthcheck)));
        assert_eq!(
            p(&["backup", "-"]),
            Ok(Run(Command::Backup(BackupTarget::Stdout)))
        );
        assert_eq!(
            p(&["backup", "/data/backups/a.db"]),
            Ok(Run(Command::Backup(BackupTarget::File(
                "/data/backups/a.db".into()
            ))))
        );
        assert_eq!(
            p(&["restore", "-"]),
            Ok(Run(Command::Restore(RestoreSource::Stdin)))
        );
        assert_eq!(
            p(&["restore", "a.db"]),
            Ok(Run(Command::Restore(RestoreSource::File("a.db".into()))))
        );
        assert_eq!(p(&["restore", "--list"]), Ok(Run(Command::RestoreList)));
        for command in ["serve", "healthcheck", "backup", "restore"] {
            assert_eq!(p(&[command, "--help"]), Ok(Help), "{command} --help");
        }
    }

    #[test]
    fn rejects_everything_else() {
        let malformed: &[&[&str]] = &[
            &[],
            &["frobnicate"],
            &["SERVE"],
            &["Serve"],
            &["-V"],
            &["--Version"],
            &["--version", "extra"],
            &["--help", "serve"],
            &["serve", "--port", "9000"],
            &["serve", "extra"],
            &["serve", "--secret", "abc"],
            &["serve", "--password", "abc"],
            &["healthcheck", "extra"],
            &["backup"],
            &["backup", ""],
            &["backup", "-x"],
            &["backup", "--list"],
            &["backup", "a.db", "b.db"],
            &["backup", "--help", "extra"],
            &["restore"],
            &["restore", "--list", "extra"],
            &["restore", "a.db", "b.db"],
            &["restore", "--LIST"],
            &["restore", "--"],
        ];
        for args in malformed {
            assert_eq!(p(args), Err(UsageError), "{args:?}");
        }
    }

    #[test]
    fn accepts_non_utf8_paths() {
        use std::os::unix::ffi::OsStringExt;
        let path = OsString::from_vec(vec![b'b', 0xff, b'.', b'd', b'b']);
        let parsed = parse([OsString::from("backup"), path.clone()]);
        assert_eq!(
            parsed,
            Ok(Invocation::Run(Command::Backup(BackupTarget::File(
                path.into()
            ))))
        );
    }

    #[test]
    fn usage_names_every_invocation() {
        for invocation in [
            "kohaku serve",
            "kohaku healthcheck",
            "kohaku backup <file>",
            "kohaku backup -",
            "kohaku restore <file>",
            "kohaku restore -",
            "kohaku restore --list",
            "kohaku --version",
            "kohaku --help",
            "kohaku <command> --help",
        ] {
            assert!(USAGE.contains(invocation), "{invocation}");
        }
    }
}
