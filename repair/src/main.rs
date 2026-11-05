//! Repo folders that were renamed or moved while herdr kept their old path in `session.json`.
//!
//! herdr has no command to change the folder of a workspace, and its server rewrites `session.json`
//! while it runs, so the only repair is to stop the server and edit the file. That is done only on an
//! explicit "yes", after listing the agents that stopping the server will close, and with a backup.
//!
//! - `check`: hook. Read-only; notifies once per repo whose folder is gone.
//! - `list`: prints every stored folder that no longer exists.
//! - `open`: action. Opens `fix` in a console window outside herdr.
//! - `show`: prints the lines of `session.json` that would change, touching nothing.
//! - `fix`: the interactive repair: backup, stop the server, edit, verify, restore on failure.

#[macro_use]
mod i18n;
mod candidate;
mod session;

use std::env;
use std::error::Error;
use std::fs;
use std::io::{self, Write};
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use candidate::Outcome;
use i18n::{is_edit, is_yes};
use serde_json::Value;
use session::{BrokenRepo, broken_repos, missing_paths, rewrite};

type Result<T> = std::result::Result<T, Box<dyn Error>>;

const STOP_WAIT: Duration = Duration::from_secs(30);

struct Env {
    herdr: String,
    /// Where this plugin keeps what it remembers between runs.
    state: PathBuf,
}

fn env() -> Env {
    Env {
        herdr: env::var("HERDR_BIN_PATH").unwrap_or_else(|_| "herdr".into()),
        state: env::var_os("HERDR_PLUGIN_STATE_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| env::temp_dir().join("herdr-toolkit"))
            .join("repair"),
    }
}

/// The `session.json` of the server this process talks to: next to its socket (a named session has its
/// own folder), or the default session's. `HERDR_REPAIR_SESSION_FILE` points at a copy, to try things.
fn session_file() -> Result<PathBuf> {
    if let Some(file) = env::var_os("HERDR_REPAIR_SESSION_FILE") {
        return Ok(file.into());
    }
    if let Some(dir) =
        env::var_os("HERDR_SOCKET_PATH").map(PathBuf::from).and_then(|s| s.parent().map(Path::to_path_buf))
    {
        return Ok(dir.join("session.json"));
    }
    let appdata = env::var_os("APPDATA").ok_or("APPDATA is not set")?;
    Ok(PathBuf::from(appdata).join("herdr").join("session.json"))
}

fn load(file: &Path) -> Result<Value> {
    let text = fs::read_to_string(file).map_err(|e| format!("cannot read {}: {e}", file.display()))?;
    Ok(serde_json::from_str(&text).map_err(|e| format!("{} is not valid JSON: {e}", file.display()))?)
}

fn exists(path: &str) -> bool {
    Path::new(path).exists()
}

fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();
    let env = env();
    let result = match args.first().map(String::as_str) {
        // A hook has nowhere to show a failure: it goes to the plugin log and herdr is told all is well.
        Some("check") => {
            if let Err(e) = check(&env) {
                eprintln!("herdr-repair: {e}");
            }
            Ok(())
        }
        Some("list") => list(),
        Some("show") => show(),
        Some("open") => open(),
        _ => fix(&env),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("herdr-repair: {e}");
            ExitCode::FAILURE
        }
    }
}

fn herdr_json(herdr: &str, args: &[&str]) -> Result<Value> {
    let output = Command::new(herdr).args(args).stdin(Stdio::null()).output()?;
    if !output.status.success() {
        return Err(format!("herdr {}: {}", args.join(" "), String::from_utf8_lossy(&output.stderr).trim()).into());
    }
    Ok(serde_json::from_slice(&output.stdout)?)
}

fn notify(herdr: &str, title: &str, body: &str) -> Result<()> {
    let output =
        Command::new(herdr).args(["notification", "show", title, "--body", body]).stdin(Stdio::null()).output()?;
    let answer: Value = serde_json::from_slice(&output.stdout)?;
    // With no herdr window in front (`no_foreground_client`) nothing is shown: it is not marked as told.
    answer["result"]["shown"].as_bool().unwrap_or(false).then_some(()).ok_or_else(|| "no notification was shown".into())
}

fn check(env: &Env) -> Result<()> {
    let proposals = proposals(&load(&session_file()?)?);
    let memory = env.state.join("notified.txt");
    let told = fs::read_to_string(&memory).unwrap_or_default();
    let mut now: Vec<&str> = Vec::new();
    for p in &proposals {
        let root = p.repo.root.as_str();
        // A plain folder with no similar one next to it was probably deleted, not renamed: not worth a notice.
        let moved_to = match &p.outcome {
            Outcome::Unique(new) | Outcome::Unverified(new) => {
                t!(" It may now be {}.", " Puede que ahora sea {}.", new.display())
            }
            Outcome::Ambiguous(_) => {
                t!(" Several folders could be its new place.", " Varias carpetas podrían ser su nuevo sitio.")
            }
            Outcome::None(_) if p.repo.repo => String::new(),
            Outcome::None(_) => continue,
        };
        if told.lines().any(|l| l == root) {
            now.push(root);
            continue;
        }
        let body = t!(
            "{root} is gone.{moved_to} Run the \"Repair moved repo folders\" action of the toolkit.",
            "{root} ya no existe.{moved_to} Ejecuta la acción \"Repair moved repo folders\" del plugin."
        );
        let title = t!("herdr: a folder was moved or renamed", "herdr: una carpeta se movió o cambió de nombre");
        if notify(&env.herdr, &title, &body).is_ok() {
            now.push(root);
        }
    }
    // Only what was told and is still broken is remembered: a folder that is fixed and breaks again is
    // told again, and one that could not be told (no herdr window open) is tried at the next event.
    fs::create_dir_all(&env.state)?;
    Ok(common::fs_util::write_atomic(&memory, now.join("\n").as_bytes())?)
}

fn list() -> Result<()> {
    let missing = missing_paths(&load(&session_file()?)?, &exists);
    if missing.is_empty() {
        println!("{}", t!("Every folder herdr remembers exists.", "Todas las carpetas que herdr recuerda existen."));
    }
    for m in missing {
        println!("{}: {} = {}", m.workspace, m.key, m.path);
    }
    Ok(())
}

/// Opens the repair in a console window of its own, outside herdr: stopping the server closes every
/// herdr pane, so a pane could not stay open to show the end of the repair. `start` gives the new
/// console to the process, and the window stays after this one exits; it inherits the environment, so
/// it finds the same server and session file.
fn open() -> Result<()> {
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    // Only the helper `cmd` is hidden; `start` opens the repair's own window.
    Command::new("cmd")
        .args(["/c", "start", "herdr repair"])
        .arg(env::current_exe()?)
        .arg("fix")
        .creation_flags(CREATE_NO_WINDOW)
        .stdin(Stdio::null())
        .spawn()?;
    Ok(())
}

/// What the repair would do for one broken repo.
struct Proposal {
    repo: BrokenRepo,
    outcome: Outcome,
}

fn proposals(doc: &Value) -> Vec<Proposal> {
    broken_repos(doc, &exists)
        .into_iter()
        .map(|repo| {
            let outcome = candidate::find(Path::new(&repo.root), &repo.checkouts, repo.repo);
            Proposal { repo, outcome }
        })
        .collect()
}

/// A linked worktree keeps a link to `<repo>/.git/worktrees/<name>`; after the repo moved, git needs
/// `worktree repair` to find it again. herdr is not involved, so this is only suggested.
fn git_repair_hints(proposal: &Proposal, new: &Path) -> Vec<String> {
    proposal
        .repo
        .checkouts
        .iter()
        .filter(|checkout| {
            let link = fs::read_to_string(Path::new(checkout).join(".git")).unwrap_or_default();
            link.strip_prefix("gitdir:").is_some_and(|dir| !Path::new(dir.trim()).exists())
        })
        .map(|checkout| format!("git -C \"{}\" worktree repair \"{checkout}\"", new.display()))
        .collect()
}

/// Takes a proposal: the move it stands for, and the git commands that go with it.
fn accept(p: &Proposal, new: &Path) -> (String, String) {
    for hint in git_repair_hints(p, new) {
        println!("{}", t!("  Afterwards, for git: {}", "  Después, para git: {}", hint));
    }
    (p.repo.root.clone(), new.to_string_lossy().into_owned())
}

/// The lines of `session.json` that hold an old folder, each with what it becomes: what the repair
/// would write, or what to change by hand.
fn print_lines(file: &Path, moves: &[(String, String)], proposals: &[Proposal]) -> Result<()> {
    let mut pairs = moves.to_vec();
    for (old, new) in moves.iter().filter(|(old, _)| proposals.iter().any(|p| p.repo.repo && p.repo.root == *old)) {
        let label = |path: &str| format!("\"label\": \"{}\"", session::last_component(path));
        pairs.push((label(old), label(new)));
    }
    println!("{}", t!("\nIn {} these lines change:", "\nEn {} cambian estas líneas:", file.display()));
    let lines = session::changed_lines(&fs::read_to_string(file)?, &pairs);
    for (number, old, new) in &lines {
        println!("{}", t!("  line {}:", "  línea {}:", number));
        println!("    - {old}\n    + {new}");
    }
    if lines.is_empty() {
        println!(
            "{}",
            t!(
                "  (none written with doubled backslashes: paths stored with forward slashes are not listed)",
                "  (ninguna escrita con barras invertidas dobles: las rutas guardadas con barras normales no salen)"
            )
        );
    }
    Ok(())
}

/// `show`: what the repair would change, without asking or touching anything.
fn show() -> Result<()> {
    let file = session_file()?;
    let proposals = proposals(&load(&file)?);
    let mut moves = Vec::new();
    for p in &proposals {
        match &p.outcome {
            Outcome::Unique(new) => moves.push((p.repo.root.clone(), new.to_string_lossy().into_owned())),
            Outcome::Unverified(new) => {
                println!(
                    "{}",
                    t!(
                        "{} -> {} (the name is all there is to go by)",
                        "{} -> {} (solo se puede ir por el nombre)",
                        p.repo.root,
                        new.display()
                    )
                );
                moves.push((p.repo.root.clone(), new.to_string_lossy().into_owned()));
            }
            Outcome::Ambiguous(found) => {
                println!(
                    "{}",
                    t!(
                        "{}: {} folders could be it, none is guessed",
                        "{}: podrían ser {} carpetas, no se adivina ninguna",
                        p.repo.root,
                        found.len()
                    )
                )
            }
            Outcome::None(why) => println!("{}", t!("{}: no proposal, {}", "{}: sin propuesta, {}", p.repo.root, why)),
        }
    }
    if moves.is_empty() {
        return Ok(());
    }
    print_lines(&file, &moves, &proposals)
}

fn agents(herdr: &str) -> Option<Vec<String>> {
    let snapshot = herdr_json(herdr, &["api", "snapshot"]).ok()?;
    let list = snapshot["result"]["snapshot"]["agents"].as_array()?;
    Some(
        list.iter()
            .map(|a| {
                format!(
                    "{} ({}) in {}",
                    a["agent"].as_str().unwrap_or("agent"),
                    a["agent_status"].as_str().unwrap_or("?"),
                    a["cwd"].as_str().unwrap_or("?")
                )
            })
            .collect(),
    )
}

fn ask(prompt: &str) -> String {
    print!("{prompt}");
    let _ = io::stdout().flush();
    let mut line = String::new();
    // No input (the pane was closed) reads as an empty answer, which is a "no".
    let _ = io::stdin().read_line(&mut line);
    line.trim().to_string()
}

fn fix(env: &Env) -> Result<()> {
    if let Err(e) = fix_interactive(env) {
        println!("\n{e}");
    }
    ask(&t!("\nPress Enter to close. ", "\nPulsa Intro para cerrar. "));
    Ok(())
}

fn server_answers(herdr: &str) -> bool {
    herdr_json(herdr, &["api", "snapshot"]).is_ok()
}

/// Copies the session file next to itself before anything touches it.
fn back_up(file: &Path) -> Result<PathBuf> {
    let stamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
    let backup = file.with_file_name(format!("session.json.pre-repair-{stamp}"));
    fs::copy(file, &backup)?;
    Ok(backup)
}

/// `herdr server stop`, and a wait until the server really is down: it saves its session while it
/// stops, so the file is only final after that.
fn stop_server(herdr: &str) -> Result<()> {
    let stopped = Command::new(herdr).args(["server", "stop"]).stdin(Stdio::null()).output()?;
    if !stopped.status.success() {
        return Err(format!("herdr server stop: {}", String::from_utf8_lossy(&stopped.stderr).trim()).into());
    }
    let mut waited = Duration::ZERO;
    while server_answers(herdr) {
        if waited >= STOP_WAIT {
            return Err(t!(
                "the herdr server is still running; nothing was changed",
                "el servidor de herdr sigue en marcha; no se ha cambiado nada"
            )
            .into());
        }
        thread::sleep(Duration::from_millis(500));
        waited += Duration::from_millis(500);
    }
    Ok(())
}

/// Rewrites the session file (server stopped) and reads it back; if that does not match, the previous
/// content is put back. Returns how many values changed.
fn write_changes(file: &Path, moves: &[(String, String)]) -> Result<usize> {
    let before = fs::read(file)?;
    let mut doc: Value = serde_json::from_slice(&before)?;
    let changed = rewrite(&mut doc, moves);
    if changed == 0 {
        return Err(t!(
            "none of the old folders is in the session file; nothing was changed",
            "ninguna de las carpetas viejas está en el fichero de sesión; no se ha cambiado nada"
        )
        .into());
    }
    let after = if before.contains(&b'\n') { serde_json::to_vec_pretty(&doc)? } else { serde_json::to_vec(&doc)? };
    common::fs_util::write_atomic(file, &after)?;
    let verified: Result<()> = (|| {
        let mut written: Value = serde_json::from_slice(&fs::read(file)?)?;
        (rewrite(&mut written, moves) == 0 && written == doc).then_some(()).ok_or_else(|| {
            t!("the file written does not read back as expected", "el fichero escrito no se lee como se esperaba")
                .into()
        })
    })();
    if let Err(e) = verified {
        common::fs_util::write_atomic(file, &before)?;
        return Err(t!("{}; the previous content was put back", "{}; se ha devuelto el contenido anterior", e).into());
    }
    Ok(changed)
}

/// The repair, in its own window. It looks for moved folders first, with the server still running (the
/// file is as complete as it needs to be to see them), and only if there is something it can repair does
/// it ask to stop the server, which closes every running agent.
fn fix_interactive(env: &Env) -> Result<()> {
    let file = session_file()?;
    let found = proposals(&load(&file)?);
    if found.is_empty() {
        println!(
            "{}",
            t!(
                "Every folder herdr remembers exists. Nothing to repair, the server is left running.",
                "Todas las carpetas que herdr recuerda existen. Nada que reparar, el servidor sigue en marcha."
            )
        );
        return Ok(());
    }
    let Some(moves) = review(&load(&file)?, &found) else {
        println!("{}", t!("The server is left running.", "El servidor sigue en marcha."));
        return Ok(());
    };
    match agents(&env.herdr) {
        Some(list) => {
            println!(
                "{}",
                t!(
                    "\nherdr's server is running. It has to be stopped to change its session file (it rewrites it\nwhile it runs). That CLOSES every running agent ({}):",
                    "\nEl servidor de herdr está en marcha. Hay que pararlo para cambiar su fichero de sesión (lo\nreescribe mientras corre). Eso CIERRA todos los agentes en marcha ({}):",
                    list.len()
                )
            );
            list.iter().for_each(|a| println!("  {a}"));
            if !is_yes(&ask(&t!("\nType yes to stop the server: ", "\nEscribe si para parar el servidor: "))) {
                println!("{}", t!("Cancelled. Nothing was changed.", "Cancelado. No se ha cambiado nada."));
                return Ok(());
            }
            println!("{}", t!("Backup: {}", "Copia de seguridad: {}", back_up(&file)?.display()));
            println!("{}", t!("Stopping the herdr server...", "Parando el servidor de herdr..."));
            stop_server(&env.herdr)?;
            println!("{}", t!("Stopped.", "Parado."));
        }
        None => println!(
            "{}",
            t!(
                "\nherdr's server is not running. Backup: {}",
                "\nEl servidor de herdr no está en marcha. Copia de seguridad: {}",
                back_up(&file)?.display()
            )
        ),
    }
    // The server saves its session as it stops: what is changed is the file as it ended up.
    print_lines(&file, &moves, &proposals(&load(&file)?))?;
    let answer = ask(&t!(
        "\nType yes to let the plugin make these changes, edit to open the file and do it yourself,\nanything else to leave it: ",
        "\nEscribe si para que el plugin haga estos cambios, editar para abrir el fichero y hacerlos tú,\ncualquier otra cosa para dejarlo: "
    ));
    if is_yes(&answer) {
        let changed = write_changes(&file, &moves)?;
        println!("{}", t!("Changed {} values in {}", "Cambiados {} valores en {}", changed, file.display()));
    } else if is_edit(&answer) {
        // `start` opens the file with whatever the user has for .json files.
        let _ = Command::new("cmd").args(["/c", "start", ""]).arg(&file).status();
        ask(&t!(
            "\nPress Enter when you have saved your changes. ",
            "\nPulsa Intro cuando hayas guardado los cambios. "
        ));
    }
    println!("{}", t!("\nDone. Open herdr again.", "\nHecho. Abre herdr de nuevo."));
    Ok(())
}

/// Prints what was found for each missing folder and asks about the ones that only match by name.
/// `None` when nothing can be repaired safely.
fn review(doc: &Value, proposals: &[Proposal]) -> Option<Vec<(String, String)>> {
    let mut moves: Vec<(String, String)> = Vec::new();
    for p in proposals {
        println!("{}", t!("\nFolder gone: {}", "\nCarpeta que falta: {}", p.repo.root));
        match &p.outcome {
            Outcome::Unique(new) => {
                println!("{}", t!("  Found it at: {}", "  Encontrada en: {}", new.display()));
                moves.push(accept(p, new));
            }
            Outcome::Unverified(new) => {
                println!(
                    "{}",
                    t!("  Only a similar name to go by: {}", "  Solo hay un nombre parecido: {}", new.display())
                );
                if p.repo.repo {
                    println!(
                        "{}",
                        t!(
                            "  (no surviving checkout of this repo to compare remotes with.)",
                            "  (no queda ningún checkout de este repo con el que comparar remotos.)"
                        )
                    );
                }
                if is_yes(&ask(&t!(
                    "  Is that the same folder? Type yes to use it: ",
                    "  ¿Es la misma carpeta? Escribe si para usarla: "
                ))) {
                    moves.push(accept(p, new));
                }
            }
            Outcome::Ambiguous(found) => {
                println!(
                    "{}",
                    t!("  Several folders fit, none is guessed:", "  Encajan varias carpetas, no se adivina ninguna:")
                );
                found.iter().for_each(|f| println!("    {}", f.display()));
            }
            Outcome::None(why) => println!("{}", t!("  No proposal: {}.", "  Sin propuesta: {}.", why)),
        }
    }
    if moves.is_empty() {
        println!(
            "{}",
            t!("\nNothing can be repaired safely by itself.", "\nNada se puede reparar con seguridad por sí solo.")
        );
        return None;
    }
    let others = missing_paths(doc, &exists).len();
    println!(
        "{}",
        t!(
            "\nStored folders that are gone, in total: {}. The ones without a proposal are left alone.",
            "\nCarpetas guardadas que faltan, en total: {}. Las que no tienen propuesta se dejan como están.",
            others
        )
    );
    Some(moves)
}
