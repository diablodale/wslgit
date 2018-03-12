use std::env;
use std::process::{Command, Stdio};
use std::io::{self, Write};

#[macro_use] extern crate lazy_static;
extern crate regex;
use regex::bytes;

// search for all occurrances of absolute DOS paths at the start of string
// this will     match on absolute DOS paths using backslashes, e.g. c:\myfile.txt
// this will     match on absolute DOS paths using foward slashes, e.g. c:/myfile.txt
// this will not match on relative paths, e.g. mydir\myfile.txt
// this will not change backslashes -> slash for relative paths, e.g. mydir/myfile.txt
// this will not work with UNC, e.g. \\server\share\path\file.txt
fn translate_path_to_unix(arg: String) -> String {
    lazy_static! {
        // can't yet force non-UTF8 with (?-u)
        static ref RE_DOSPATH: regex::Regex = regex::Regex::new(r"^([A-Za-z]):((?:\\|/).*)$").unwrap();
    }
    let result = RE_DOSPATH.replace(&arg, |caps: &regex::Captures| {
        // preallocate a String with the known size
        let mut new_path: String = String::with_capacity(caps[2].len() + 6);
        // construct the WSL path
        new_path.push_str("/mnt/");
        new_path.push_str(&caps[1].to_ascii_lowercase());
        new_path.push_str(&caps[2].replace("\\", "/"));
        return new_path;
    });
    return result.into_owned();
}

fn shell_escape(arg: String) -> String {
    // ToDo: This really only handles arguments with spaces.
    // More complete shell escaping is required for the general case.
    if arg.contains(" ") {
        return vec![
            String::from("\""),
            arg,
            String::from("\"")].join("");
    }
    arg
}

fn main() {
    let mut cmd_args = Vec::new();
    let mut git_args: Vec<String> = vec![String::from("git")];
    let git_cmd: String;

    // check for advanced usage indicated by BASH_ENV and WSLENV=BASH_ENV
    let mut interactive_shell = true;
    if env::var("BASH_ENV").is_ok() {
        let wslenv = env::var("WSLENV");
        if wslenv.is_ok() && wslenv.unwrap().split(':').position(|r| r.eq_ignore_ascii_case("BASH_ENV")).is_some() {
            interactive_shell = false;
        }
    }

    // process git command arguments
    if interactive_shell {
        git_args.extend(env::args().skip(1)
            .map(translate_path_to_unix)
            .map(shell_escape));
        git_cmd = git_args.join(" ");
        cmd_args.push("bash".to_string());
        cmd_args.push("-ic".to_string());
        cmd_args.push(git_cmd.clone());
    }
    else {
        git_args.extend(env::args().skip(1)
        .map(translate_path_to_unix));
        git_cmd = git_args.join(" ");
        cmd_args.clone_from(&git_args);
    }

    // setup stdin/stdout
    let stdin_mode = if git_cmd.ends_with("--version") {
        // For some reason, the git subprocess seems to hang, waiting for 
        // input, when VS Code 1.17.2 tries to detect if `git --version` works
        // on Windows 10 1709 (specifically, in `findSpecificGit` in the
        // VS Code source file `extensions/git/src/git.ts`).
        // To workaround this, we only pass stdin to the git subprocess
        // for all other commands, but not for the initial `--version` check.
        // Stdin is needed for example when commiting, where the commit
        // message is passed on stdin.
        Stdio::null()
    } else {
        Stdio::inherit()
    };

    // setup the git subprocess launched inside WSL
    let mut git_proc_setup = Command::new("wsl");
    git_proc_setup.args(&cmd_args)
        .stdin(stdin_mode);
    
    // add git commands that must skip translate_path_to_win and
    // transparently pass-through bytes of data with no charset
    // validation or conversion
    // e.g. = &["show", "status, "rev-parse", "for-each-ref"];
    const NO_TRANSLATE: &'static [&'static str] = &["show"];

    // write any stdout
    let status = if (git_args.len() > 1) && (NO_TRANSLATE.iter().position(|&r| r == git_args[1]).is_none()) {
        // run the subprocess and capture its output
        let git_proc = git_proc_setup
            .stdout(Stdio::piped())
            .spawn()
            .expect(&format!("Failed to execute command '{}'", &git_cmd));
        let output = git_proc
            .wait_with_output()
            .expect(&format!("Failed to wait for git call '{}'", &git_cmd));

        // search for all occurrances of *nix paths at the start of any line
        lazy_static! {
            // Rust stdio demands utf-8 via vec<u8>, don't need to parse so can use faster non utf-8 regex engine
            static ref RE_WSLPATH: bytes::Regex = bytes::Regex::new(r"(?m-u)^/mnt/([A-Za-z])(/.*)$").unwrap();
        }
        let result = RE_WSLPATH.replace_all(&output.stdout, |caps: &bytes::Captures| {
            // preallocate a vector with the known size
            let mut new_path: Vec<u8> = Vec::with_capacity(caps[2].len() + 2);
            // construct the DOS path
            new_path.push(caps[1][0].to_ascii_uppercase());
            new_path.push(b':');
            new_path.extend_from_slice(&caps[2]);
            return new_path;
        });
        io::stdout().write_all(&result).unwrap();

        // std::process::exit does not call destructors; must manually flush
        io::stdout().flush().unwrap();

        // return status of child process
        output.status
    }
    else {
        // run the subprocess without capturing its output
        // the output of the subprocess is passed through unchanged
        git_proc_setup
            .status()
            .expect(&format!("Failed to execute command '{}'", &git_cmd))
    };

    // forward any exit code
    if let Some(exit_code) = status.code() {
        std::process::exit(exit_code);
    }
}
