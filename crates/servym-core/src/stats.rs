//! Processeur et mémoire des projets.
//!
//! Un relevé liste tous les processus de la machine : PID → [`ProcSample`]. Un projet compte toute
//! son arborescence (cmd.exe, puis node et ses propres enfants).

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::Duration;

use serde::Serialize;
pub use servym_sys::{ProcSample, ProcTable, tree_of};
use tokio::sync::Mutex;

use crate::BoxFuture;

/// Relevé daté (ms).
#[derive(Clone, Debug, Default)]
pub struct Snapshot {
    pub at: i64,
    pub processes: ProcTable,
}

/// Usage d'une arborescence.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Usage {
    /// Part de toute la machine (100 : tous les cœurs), au dixième ; `None` au premier relevé.
    pub cpu: Option<f64>,
    /// Octets.
    pub memory: u64,
    pub processes: usize,
}

/// Usage d'un ensemble de PID déjà établi, en part de toute la machine, comme le Gestionnaire
/// des tâches ; `None` si l'ensemble est vide.
fn usage_of_tree(
    prev: Option<&Snapshot>,
    next: &Snapshot,
    tree: &BTreeSet<u32>,
    cores: u32,
) -> Option<Usage> {
    if tree.is_empty() {
        return None;
    }
    let mut memory = 0;
    let mut used = 0;
    for pid in tree {
        let p = &next.processes[pid];
        memory += p.memory;
        match prev.and_then(|s| s.processes.get(pid)) {
            Some(before) if before.start == p.start => used += p.cpu.saturating_sub(before.cpu),
            // Démarré depuis le relevé précédent : compté depuis son démarrage.
            _ if prev.is_some_and(|s| p.start >= s.at) => used += p.cpu,
            _ => {}
        }
    }
    let elapsed = prev.map_or(0, |s| next.at - s.at);
    let cpu = (elapsed > 0).then(|| {
        let share = used as f64 / (elapsed as f64 * f64::from(cores.max(1)));
        ((share * 1000.0 + 0.5).floor() / 10.0).min(100.0)
    });
    Some(Usage {
        cpu,
        memory,
        processes: tree.len(),
    })
}

/// Usage d'une arborescence entre deux relevés ; `None` si aucun processus n'est trouvé.
pub fn usage_of(prev: Option<&Snapshot>, next: &Snapshot, roots: &[u32], cores: u32) -> Option<Usage> {
    usage_of_tree(prev, next, &tree_of(&next.processes, roots), cores)
}

/// Fonction de relevé des processus (le système, ou une fonction factice dans les tests).
pub type SampleFn = Arc<dyn Fn() -> BoxFuture<'static, ProcTable> + Send + Sync>;
/// Horloge en millisecondes.
pub type NowFn = Arc<dyn Fn() -> i64 + Send + Sync>;

/// Mesures de toutes les racines, de Servym lui-même et du total.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct MeasureAll {
    pub projects: BTreeMap<String, Usage>,
    /// Servym sans les arborescences des projets.
    #[serde(rename = "self")]
    pub own: Option<Usage>,
    /// Servym et ses projets, chaque processus une fois.
    pub total: Option<Usage>,
}

#[derive(Default)]
struct Snapshots {
    prev: Option<Snapshot>,
    last: Option<Snapshot>,
}

/// Relevés à la demande, partagés : plusieurs pages ouvertes (ou deux requêtes simultanées) dans
/// l'intervalle `max_age` réutilisent la même mesure.
pub struct Sampler {
    sample: SampleFn,
    now: NowFn,
    cores: u32,
    max_age: i64,
    // Tenu pendant la mesure : une seconde demande attend la première et réutilise son relevé.
    snapshots: Mutex<Snapshots>,
}

impl Sampler {
    pub fn new(sample: SampleFn, now: NowFn, cores: u32, max_age: Duration) -> Self {
        Sampler {
            sample,
            now,
            cores,
            max_age: max_age.as_millis() as i64,
            snapshots: Mutex::default(),
        }
    }

    /// Usage de chaque racine ; une racine introuvable est absente du résultat.
    pub async fn measure(&self, roots: &BTreeMap<String, Vec<u32>>) -> BTreeMap<String, Usage> {
        self.measure_inner(roots, None).await.projects
    }

    /// Comme `measure`, plus Servym (sans ses projets) et le total, chaque processus une fois.
    pub async fn measure_all(&self, roots: &BTreeMap<String, Vec<u32>>, own_pid: u32) -> MeasureAll {
        self.measure_inner(roots, Some(own_pid)).await
    }

    async fn measure_inner(&self, roots: &BTreeMap<String, Vec<u32>>, own_pid: Option<u32>) -> MeasureAll {
        let mut snaps = self.snapshots.lock().await;
        let fresh = snaps
            .last
            .as_ref()
            .is_some_and(|l| (self.now)() - l.at < self.max_age);
        if !fresh {
            let at = (self.now)();
            let processes = (self.sample)().await;
            snaps.prev = snaps.last.take();
            snaps.last = Some(Snapshot { at, processes });
        }
        let last = snaps.last.as_ref().unwrap();
        let prev = snaps.prev.as_ref();
        let mut projects = BTreeMap::new();
        let mut in_projects = BTreeSet::new();
        for (id, pids) in roots {
            let tree = tree_of(&last.processes, pids);
            in_projects.extend(tree.iter().copied());
            if let Some(usage) = usage_of_tree(prev, last, &tree, self.cores) {
                projects.insert(id.clone(), usage);
            }
        }
        let Some(own_pid) = own_pid else {
            return MeasureAll {
                projects,
                own: None,
                total: None,
            };
        };
        let own: BTreeSet<u32> = tree_of(&last.processes, &[own_pid])
            .into_iter()
            .filter(|pid| !in_projects.contains(pid))
            .collect();
        let all: BTreeSet<u32> = own.union(&in_projects).copied().collect();
        MeasureAll {
            projects,
            own: usage_of_tree(prev, last, &own, self.cores),
            total: usage_of_tree(prev, last, &all, self.cores),
        }
    }
}
