// Processeur et mémoire des projets. Un relevé liste tous les processus de la
// machine (src/system.js) : PID → { pid, ppid, start, cpu, memory }, avec start
// l'heure de démarrage (ms), cpu le temps processeur cumulé (ms) et memory la
// mémoire vive utilisée (octets). Un projet compte toute son arborescence :
// cmd.exe, puis node et ses propres enfants.

// PID de l'arborescence de chaque racine. Un enfant démarré avant son parent
// désigne un ancien processus au PID réutilisé : il n'en fait pas partie.
export function treeOf(processes, roots) {
  const children = new Map();
  for (const p of processes.values()) {
    if (!children.has(p.ppid)) children.set(p.ppid, []);
    children.get(p.ppid).push(p);
  }
  const tree = new Set();
  const visit = (pid) => {
    if (tree.has(pid) || !processes.has(pid)) return;
    tree.add(pid);
    for (const child of children.get(pid) ?? []) {
      if (child.pid !== pid && child.start >= processes.get(pid).start) visit(child.pid);
    }
  };
  for (const pid of roots) visit(pid);
  return tree;
}

// Usage d'une arborescence entre deux relevés, en part de toute la machine
// (100 % : tous les cœurs occupés), comme le Gestionnaire des tâches. Sans
// relevé précédent, le processeur est inconnu (null). null : aucun processus.
export function usageOf(prev, next, roots, cores) {
  const tree = treeOf(next.processes, roots);
  if (!tree.size) return null;
  let memory = 0;
  let used = 0;
  for (const pid of tree) {
    const p = next.processes.get(pid);
    memory += p.memory;
    const before = prev?.processes.get(pid);
    if (before && before.start === p.start) used += Math.max(0, p.cpu - before.cpu);
    else if (prev && p.start >= prev.at) used += p.cpu; // démarré depuis le relevé précédent
  }
  const elapsed = prev ? next.at - prev.at : 0;
  const cpu = elapsed > 0 ? Math.min(100, Math.round((used / (elapsed * cores)) * 1000) / 10) : null;
  return { cpu, memory, processes: tree.size };
}

// Relevés à la demande, partagés : plusieurs pages ouvertes (ou deux requêtes
// simultanées) dans l'intervalle maxAge réutilisent la même mesure.
export class Sampler {
  #sample;
  #now;
  #cores;
  #maxAge;
  #prev = null;
  #last = null;
  #pending = null;

  constructor({ sample, now = () => Date.now(), cores, maxAge = 2000 }) {
    this.#sample = sample;
    this.#now = now;
    this.#cores = cores;
    this.#maxAge = maxAge;
  }

  async #snapshot() {
    if (this.#last && this.#now() - this.#last.at < this.#maxAge) return;
    this.#pending ??= (async () => {
      const at = this.#now();
      const processes = await this.#sample();
      this.#prev = this.#last;
      this.#last = { at, processes };
    })().finally(() => (this.#pending = null));
    await this.#pending;
  }

  // rootsById : { id: [pid, …] } → { id: { cpu, memory, processes } }, sans les projets introuvables.
  async measure(rootsById) {
    await this.#snapshot();
    const result = {};
    for (const [id, roots] of Object.entries(rootsById)) {
      const usage = usageOf(this.#prev, this.#last, roots, this.#cores);
      if (usage) result[id] = usage;
    }
    return result;
  }
}
