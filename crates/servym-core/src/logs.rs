//! Journaux des projets : un fichier par projet, lu par la fin et suivi en direct ; historique des
//! changements d'état et journal d'audit, lus dans `servym.log`.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::LazyLock;
#[cfg(not(unix))]
use std::time::SystemTime;

use regex::Regex;
use serde::Serialize;

/// Lignes gardées à l'affichage.
pub const MAX_LINES: usize = 500;

static ANSI: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\x1b\[[0-9;?]*[ -/]*[@-~]").unwrap());
static CONTROL: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[\x00-\x08\x0b-\x1f]").unwrap());
/// Même motif que `public/js/log-patterns.js` (l’interface colore les mêmes lignes). cmd.exe écrit
/// « n’est pas reconnu » avec une apostrophe droite ou typographique selon son encodage.
static ERROR_LINE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)error|erreur|exception|introuvable|n['’]est pas reconnu|is not recognized|EADDRINUSE|EACCES|cannot|impossible|failed|fatal").unwrap()
});
/// Avertissements : jamais choisis comme cause d'un arrêt (une dépréciation après la vraie erreur la
/// masquerait).
static WARN_LINE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)warn|avertissement").unwrap());

/// Ligne sans codes ANSI ni caractères de contrôle (sauf la tabulation).
pub fn clean(line: &str) -> String {
    CONTROL.replace_all(&ANSI.replace_all(line, ""), "").into_owned()
}

/// Fin d'un fichier : lignes complètes (la première, coupée, est retirée) et taille.
fn read_end(file: &Path, bytes: u64) -> (Vec<String>, u64) {
    let read = || -> std::io::Result<(Vec<String>, u64)> {
        let mut f = File::open(file)?;
        let size = f.metadata()?.len();
        let start = size.saturating_sub(bytes);
        f.seek(SeekFrom::Start(start))?;
        let mut buf = Vec::with_capacity((size - start) as usize);
        f.take(size - start).read_to_end(&mut buf)?;
        let text = String::from_utf8_lossy(&buf);
        let mut lines: Vec<&str> = text
            .split('\n')
            .map(|l| l.strip_suffix('\r').unwrap_or(l))
            .collect();
        if start > 0 {
            lines.remove(0);
        }
        if lines.last() == Some(&"") {
            lines.pop();
        }
        Ok((lines.into_iter().map(clean).collect(), size))
    };
    read().unwrap_or((Vec::new(), 0))
}

/// Dernières lignes d'un journal (codes ANSI retirés).
pub fn tail_lines(file: &Path, count: usize) -> Vec<String> {
    let (lines, _) = read_end(file, 96_000);
    let skip = lines.len().saturating_sub(count);
    lines.into_iter().skip(skip).collect()
}

fn own_lines(file: &Path) -> Vec<String> {
    tail_lines(file, 60)
        .into_iter()
        .filter(|l| !l.trim().is_empty() && !l.starts_with("[servym]"))
        .collect()
}

/// Texte coupé à 300 unités UTF-16, comme `slice(0, 300)`.
fn short(line: &str) -> String {
    let mut out = String::new();
    let mut units = 0;
    for c in line.trim().chars() {
        units += c.len_utf16();
        if units > 300 {
            break;
        }
        out.push(c);
    }
    out
}

/// Ligne la plus parlante pour expliquer un arrêt : la dernière erreur, sinon la dernière ligne.
pub fn last_error(file: &Path) -> Option<String> {
    let lines = own_lines(file);
    lines
        .iter()
        .rev()
        .find(|l| ERROR_LINE.is_match(l))
        .or(lines.last())
        .map(|l| short(l))
}

/// Programme sorti sans erreur pendant son lancement : sa dernière erreur, sinon son dernier
/// avertissement (« The local web server is already running »), sinon rien : la dernière ligne
/// seule n'expliquerait rien (« None »).
pub fn startup_reason(file: &Path) -> Option<String> {
    let lines = own_lines(file);
    lines
        .iter()
        .rev()
        .find(|l| ERROR_LINE.is_match(l))
        .or_else(|| lines.iter().rev().find(|l| WARN_LINE.is_match(l)))
        .map(|l| short(l))
}

// ---- Historique ----

/// Changement d'état : « <date> [state] <id> <avant> → <après>[ · détail] » ([état] dans les
/// journaux écrits avant la traduction).
static TRANSITION: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^(\S+) \[(?:state|état)\] ([a-z0-9-]+) (\w+) → (\w+)(?: · (.*))?$").unwrap()
});
/// Audit : « <date> [audit] <projet ou -> <compte ou -> · <texte> ».
static AUDIT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(\S+) \[audit\] ([a-z0-9-]+|-) (\S+) · (.*)$").unwrap());

/// Entrée de l'historique (changement d'état ou audit), au format de l'API.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(untagged)]
pub enum HistoryEntry {
    Transition {
        at: String,
        id: String,
        from: String,
        to: String,
        detail: String,
    },
    Audit {
        at: String,
        id: Option<String>,
        kind: &'static str,
        user: Option<String>,
        text: String,
    },
}

/// Historique d'un projet (de tous, sans `id`), lu dans les journaux donnés (du plus ancien au plus
/// récent) ; rendu du plus récent au plus ancien, `max` entrées au plus.
pub fn history(files: &[PathBuf], id: Option<&str>, max: usize) -> Vec<HistoryEntry> {
    let mut entries = Vec::new();
    for file in files {
        for line in read_end(file, 1 << 20).0 {
            if let Some(m) = TRANSITION.captures(&line) {
                if id.is_none_or(|id| id == &m[2]) {
                    entries.push(HistoryEntry::Transition {
                        at: m[1].into(),
                        id: m[2].into(),
                        from: m[3].into(),
                        to: m[4].into(),
                        detail: m.get(5).map_or("", |d| d.as_str()).into(),
                    });
                }
                continue;
            }
            if let Some(a) = AUDIT.captures(&line)
                && id.is_none_or(|id| id == &a[2])
            {
                let opt = |s: &str| (s != "-").then(|| s.to_string());
                entries.push(HistoryEntry::Audit {
                    at: a[1].into(),
                    id: opt(&a[2]),
                    kind: "audit",
                    user: opt(&a[3]),
                    text: a[4].into(),
                });
            }
        }
    }
    let skip = entries.len().saturating_sub(max);
    entries.into_iter().skip(skip).rev().collect()
}

// ---- Suivi en direct ----

/// Évènement du suivi d'un journal.
#[derive(Clone, Debug, PartialEq)]
pub enum Follow {
    /// Journal remplacé (nouveau lancement) ou recommencé : toutes ses dernières lignes.
    Reset(Vec<String>),
    /// Nouvelles lignes complètes.
    Lines(Vec<String>),
}

const MAX_PENDING: usize = 256 * 1024;

#[cfg(unix)]
type FileId = (u64, u64);
#[cfg(not(unix))]
type FileId = SystemTime;

fn file_id(meta: &std::fs::Metadata) -> Option<FileId> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        Some((meta.dev(), meta.ino()))
    }
    #[cfg(not(unix))]
    {
        meta.created().ok()
    }
}

/// Suivi d'un journal par relectures successives (`poll`).
pub struct Follower {
    file: PathBuf,
    pos: u64,
    identity: Option<FileId>,
    pending: Vec<u8>,
}

impl Follower {
    /// Commence le suivi ; rend aussi les dernières lignes (évènement `Reset`).
    pub fn new(file: &Path) -> (Follower, Follow) {
        let mut follower = Follower {
            file: file.to_path_buf(),
            pos: 0,
            identity: None,
            pending: Vec::new(),
        };
        let first = follower.reset();
        (follower, first)
    }

    fn reset(&mut self) -> Follow {
        let (lines, size) = read_end(&self.file, 96_000);
        self.pos = size;
        self.identity = std::fs::metadata(&self.file).ok().and_then(|m| file_id(&m));
        self.pending.clear();
        let skip = lines.len().saturating_sub(MAX_LINES);
        Follow::Reset(lines.into_iter().skip(skip).collect())
    }

    /// Ce qui a changé depuis la dernière lecture.
    pub fn poll(&mut self) -> Option<Follow> {
        let Ok(info) = std::fs::metadata(&self.file) else {
            return (self.identity.is_some() || self.pos > 0).then(|| self.reset());
        };
        let size = info.len();
        if file_id(&info) != self.identity || size < self.pos {
            return Some(self.reset());
        }
        if size == self.pos {
            return None;
        }
        let mut buf = Vec::new();
        let read = File::open(&self.file).and_then(|mut f| {
            f.seek(SeekFrom::Start(self.pos))?;
            f.take((size - self.pos).min(1 << 20)).read_to_end(&mut buf)
        });
        if read.is_err() {
            return None;
        }
        self.pos += buf.len() as u64;
        self.pending.extend_from_slice(&buf);
        // Lignes complètes seulement ; un caractère UTF-8 coupé attend la suite (sauf ligne sans fin bornée).
        let complete: Vec<u8> = if let Some(last) = self.pending.iter().rposition(|b| *b == b'\n') {
            self.pending.drain(..=last).collect()
        } else if self.pending.len() >= MAX_PENDING {
            let mut flushed = std::mem::take(&mut self.pending);
            flushed.push(b'\n');
            flushed
        } else {
            return None;
        };
        let text = String::from_utf8_lossy(&complete);
        let lines: Vec<String> = text
            .split('\n')
            .map(|l| l.strip_suffix('\r').unwrap_or(l))
            .map(clean)
            .collect::<Vec<_>>();
        let lines = &lines[..lines.len() - 1]; // après le dernier saut de ligne : vide
        let skip = lines.len().saturating_sub(MAX_LINES);
        Some(Follow::Lines(lines[skip..].to_vec()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn suivi_lignes_puis_remplacement() {
        let dir = std::env::temp_dir().join(format!("servym-follow-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("p.log");
        std::fs::write(&file, "un\n\x1b[32mdeux\x1b[39m\n").unwrap();
        let (mut f, first) = Follower::new(&file);
        assert_eq!(first, Follow::Reset(vec!["un".into(), "deux".into()]));
        assert_eq!(f.poll(), None);
        let mut append = std::fs::OpenOptions::new().append(true).open(&file).unwrap();
        std::io::Write::write_all(&mut append, "trois\r\nqua".as_bytes()).unwrap();
        assert_eq!(f.poll(), Some(Follow::Lines(vec!["trois".into()])));
        std::io::Write::write_all(&mut append, "tre ✓\n".as_bytes()).unwrap();
        assert_eq!(f.poll(), Some(Follow::Lines(vec!["quatre ✓".into()])));
        drop(append);
        std::fs::write(&file, "nouveau\n").unwrap();
        assert_eq!(f.poll(), Some(Follow::Reset(vec!["nouveau".into()])));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn historique_du_plus_recent_au_plus_ancien() {
        let dir = std::env::temp_dir().join(format!("servym-history-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("servym.log");
        std::fs::write(
            &file,
            "2026-01-01T00:00:00Z [state] a stopped → starting\n\
             2026-01-01T00:00:01Z [état] a starting → failed · code 3\n\
             2026-01-01T00:00:02Z [audit] - alice · Protection activée\n\
             2026-01-01T00:00:03Z [state] b stopped → starting\n",
        )
        .unwrap();
        let all = history(std::slice::from_ref(&file), None, 100);
        assert_eq!(all.len(), 4);
        assert!(matches!(&all[0], HistoryEntry::Transition { id, .. } if id == "b"));
        let a = history(std::slice::from_ref(&file), Some("a"), 1);
        assert_eq!(
            a,
            [HistoryEntry::Transition {
                at: "2026-01-01T00:00:01Z".into(),
                id: "a".into(),
                from: "starting".into(),
                to: "failed".into(),
                detail: "code 3".into()
            }]
        );
        let audit = serde_json::to_value(&all[1]).unwrap();
        assert_eq!(audit["kind"], "audit");
        assert_eq!(audit["id"], serde_json::Value::Null);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
