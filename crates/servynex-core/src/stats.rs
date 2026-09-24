//! Processeur et mémoire des projets.
//!
//! Un relevé liste tous les processus de la machine : PID → [`ProcSample`]. Un projet compte toute
//! son arborescence (cmd.exe, puis node et ses propres enfants).

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::Duration;

use serde::Serialize;

pub use servynex_sys::{ProcSample, ProcTable};

use crate::BoxFuture;

/// Relevé daté (ms).
#[derive(Clone, Debug, Default)]
pub struct Snapshot {
    pub at: i64,
    pub processes: ProcTable,
}

/// PID de l'arborescence de chaque racine. Un enfant démarré avant son parent désigne un ancien
/// processus au PID réutilisé : il n'en fait pas partie.
pub fn tree_of(processes: &ProcTable, roots: &[u32]) -> BTreeSet<u32> {
    let _ = (processes, roots);
    unimplemented!("phase 2")
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

/// Usage d'une arborescence entre deux relevés ; `None` si aucun processus n'est trouvé.
pub fn usage_of(prev: Option<&Snapshot>, next: &Snapshot, roots: &[u32], cores: u32) -> Option<Usage> {
    let _ = (prev, next, roots, cores);
    unimplemented!("phase 2")
}

/// Fonction de relevé des processus (le système, ou une fonction factice dans les tests).
pub type SampleFn = Arc<dyn Fn() -> BoxFuture<'static, ProcTable> + Send + Sync>;
/// Horloge en millisecondes.
pub type NowFn = Arc<dyn Fn() -> i64 + Send + Sync>;

/// Mesures de toutes les racines, de Servynex lui-même et du total.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct MeasureAll {
    pub projects: BTreeMap<String, Usage>,
    #[serde(rename = "self")]
    pub own: Usage,
    pub total: Usage,
}

/// Relevés à la demande, partagés : plusieurs pages ouvertes (ou deux requêtes simultanées) dans
/// l'intervalle `max_age` réutilisent la même mesure.
pub struct Sampler {
    _private: (),
}

impl Sampler {
    pub fn new(sample: SampleFn, now: NowFn, cores: u32, max_age: Duration) -> Self {
        let _ = (sample, now, cores, max_age);
        unimplemented!("phase 2")
    }

    /// Usage de chaque racine ; une racine introuvable est absente du résultat.
    pub async fn measure(&self, roots: &BTreeMap<String, Vec<u32>>) -> BTreeMap<String, Usage> {
        let _ = roots;
        unimplemented!("phase 2")
    }

    /// Comme `measure`, plus Servynex (sans ses projets) et le total, chaque processus une fois.
    pub async fn measure_all(&self, roots: &BTreeMap<String, Vec<u32>>, own_pid: u32) -> MeasureAll {
        let _ = (roots, own_pid);
        unimplemented!("phase 2")
    }
}
