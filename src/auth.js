// Authentification facultative : comptes, permissions, sessions, verrouillage
// après inactivité, réinitialisation et clés de secours, invitations.
//
// Sans runtime/auth.json, ou protection désactivée : tout est ouvert (usage local
// à une personne). Protection active : seule la page de connexion répond tant
// qu'on n'est pas connecté. Le fichier est relu s'il change sur le disque
// (commande « npm run auth »). Illisible alors que la protection était active :
// tout est refusé (échec fermé) jusqu'à ce qu'on le corrige ou le supprime.
import crypto from 'node:crypto';
import fs from 'node:fs';
import path from 'node:path';
import { promisify } from 'node:util';

const scryptAsync = promisify(crypto.scrypt);

// ---- Permissions ----

// view : tout voir · operate : démarrer, arrêter, ouvrir · edit : réglages sans
// risque · commands : commande, dossier, environnement, exposition, ajout de
// projet (autant dire un accès complet à la machine) · instance : redémarrer Servynex,
// démarrage avec Windows, nom de l'instance · admin : tout, comptes et sécurité.
export const PERMISSIONS = ['view', 'operate', 'edit', 'commands', 'instance', 'admin'];
export const ROLES = {
  readonly: ['view'],
  operator: ['view', 'operate'],
  editor: ['view', 'operate', 'edit'],
  admin: [...PERMISSIONS],
};
export function roleOf(permissions) {
  if (permissions.includes('admin')) return 'admin';
  const key = [...permissions].sort().join();
  return Object.entries(ROLES).find(([, p]) => [...p].sort().join() === key)?.[0] ?? 'custom';
}
function normalizePermissions(list) {
  if (!Array.isArray(list) || !list.every((p) => PERMISSIONS.includes(p))) throw new AuthError(400, 'Permissions inconnues');
  if (list.includes('admin')) return [...PERMISSIONS];
  const set = new Set(['view', ...list]);
  if (set.has('commands')) set.add('edit');
  if (set.has('edit')) set.add('operate');
  return PERMISSIONS.filter((p) => set.has(p));
}

const ROLE_NAMES = { admin: 'administrateur', editor: 'éditeur', operator: 'opérateur', readonly: 'lecture seule', custom: 'permissions personnalisées' };
const roleName = (permissions) => ROLE_NAMES[roleOf(permissions)];

// Verrouillage après inactivité (secondes, 0 : jamais).
export const LOCK_CHOICES = [0, 120, 300, 900, 1800, 3600];

const SESSION_SHORT = 12 * 3600e3; // sans « rester connecté » : 12 h sans activité
const SESSION_LONG = 30 * 86400e3;
const RESET_TTL = 15 * 60e3;
const INVITE_TTL = 48 * 3600e3;
const MAX_SESSIONS = 20;
const USERNAME = /^[a-z0-9][a-z0-9._-]{1,31}$/;
const COOKIE = 'servynex_session';

export class AuthError extends Error {
  constructor(status, message, extra = {}) {
    super(message);
    this.status = status;
    this.extra = extra;
  }
}

// ---- Secrets ----

const SCRYPT = { N: 32768, r: 8, p: 1, maxmem: 64 * 1024 * 1024 };

export async function hashSecret(secret) {
  const salt = crypto.randomBytes(16);
  const hash = await scryptAsync(String(secret).normalize('NFKC'), salt, 64, SCRYPT);
  return `scrypt$${SCRYPT.N}$${SCRYPT.r}$${SCRYPT.p}$${salt.toString('base64')}$${hash.toString('base64')}`;
}

export async function verifySecret(secret, stored) {
  const parts = typeof stored === 'string' ? stored.split('$') : [];
  if (parts.length !== 6 || parts[0] !== 'scrypt') {
    await hashSecret('x'); // même durée qu'une vraie vérification
    return false;
  }
  const [, N, r, p, salt, hash] = parts;
  const expected = Buffer.from(hash, 'base64');
  const actual = await scryptAsync(String(secret).normalize('NFKC'), Buffer.from(salt, 'base64'), expected.length, { N: Number(N), r: Number(r), p: Number(p), maxmem: SCRYPT.maxmem });
  return crypto.timingSafeEqual(actual, expected);
}

const sha256 = (s) => crypto.createHash('sha256').update(String(s)).digest('hex');
const randomToken = () => crypto.randomBytes(32).toString('base64url');
const ALPHABET = 'ABCDEFGHJKLMNPQRSTUVWXYZ23456789'; // sans 0/O ni 1/I

// « 7KQF-M2XA-… » : groupes de 4 caractères faciles à recopier.
export function humanCode(groups) {
  const bytes = crypto.randomBytes(groups * 4);
  const chars = [...bytes].map((b) => ALPHABET[b % ALPHABET.length]);
  return Array.from({ length: groups }, (_, i) => chars.slice(i * 4, i * 4 + 4).join('')).join('-');
}
const normalizeCode = (s) => String(s ?? '').toUpperCase().replace(/[^A-Z0-9]/g, '');

function checkPassword(password) {
  if (typeof password !== 'string' || password.length < 8) throw new AuthError(400, 'Mot de passe trop court : 8 caractères au moins.');
  if (password.length > 200) throw new AuthError(400, 'Mot de passe trop long.');
}
function checkUsername(username) {
  if (typeof username !== 'string' || !USERNAME.test(username)) {
    throw new AuthError(400, 'Nom d’utilisateur : 2 à 32 caractères, minuscules, chiffres, point, tiret ou tiret bas.');
  }
}
function checkName(name) {
  if (name !== undefined && (typeof name !== 'string' || name.trim().length > 60)) throw new AuthError(400, 'Nom affiché trop long.');
}

// ---- Stockage : runtime/auth.json ----

const empty = () => ({ version: 1, enabled: false, lockAfter: 0, users: [], sessions: [], invites: [], resets: [] });

export class AuthStore {
  constructor(file) {
    this.file = file;
    this.mtime = null;
    this.error = null;
    this.data = empty();
    this.load();
  }

  // Relit le fichier s'il a changé (commande npm run auth, modification à la main).
  load() {
    let info;
    try {
      info = fs.statSync(this.file);
    } catch {
      if (this.mtime !== null || this.error) Object.assign(this, { data: empty(), mtime: null, error: null });
      return this.data;
    }
    if (info.mtimeMs === this.mtime) return this.data;
    try {
      this.data = { ...empty(), ...JSON.parse(fs.readFileSync(this.file, 'utf8')) };
      this.error = null;
    } catch (err) {
      this.error = `runtime/auth.json illisible : ${err.message}`;
    }
    this.mtime = info.mtimeMs;
    return this.data;
  }

  save() {
    fs.mkdirSync(path.dirname(this.file), { recursive: true });
    const tmp = `${this.file}.tmp`;
    fs.writeFileSync(tmp, `${JSON.stringify(this.data, null, 2)}\n`);
    fs.renameSync(tmp, this.file);
    this.mtime = fs.statSync(this.file).mtimeMs;
  }
}

// Code de réinitialisation valable 15 minutes (commande npm run auth, ou un administrateur).
export function createResetCode(store, username, now = Date.now()) {
  const data = store.load();
  const user = data.users.find((u) => u.username === username);
  if (!user) throw new AuthError(404, `Compte inconnu : ${username}`);
  const code = humanCode(2);
  data.resets = data.resets.filter((r) => r.userId !== user.id && r.expiresAt > now);
  data.resets.push({ userId: user.id, codeHash: sha256(normalizeCode(code)), expiresAt: now + RESET_TTL });
  store.save();
  return code;
}

// ---- Petits outils HTTP ----

function parseCookies(req) {
  const out = {};
  for (const part of String(req.headers.cookie ?? '').split(';')) {
    const i = part.indexOf('=');
    if (i > 0) out[part.slice(0, i).trim()] = part.slice(i + 1).trim();
  }
  return out;
}

function describeAgent(ua = '') {
  const browser = /Edg\//.test(ua) ? 'Edge' : /OPR\//.test(ua) ? 'Opera' : /Firefox\//.test(ua) ? 'Firefox' : /Chrome\//.test(ua) ? 'Chrome' : /Safari\//.test(ua) ? 'Safari' : 'Navigateur';
  const os = /Windows/.test(ua) ? 'Windows' : /Android/.test(ua) ? 'Android' : /iPhone|iPad/.test(ua) ? 'iOS' : /Mac OS/.test(ua) ? 'macOS' : /Linux/.test(ua) ? 'Linux' : '';
  return os ? `${browser} · ${os}` : browser;
}

const clientIp = (req) => req.socket?.remoteAddress ?? '?';

// ---- Service ----

export class Auth {
  #failures = new Map(); // clé → { count, until }
  #seenSaved = new Map(); // session → dernière écriture de lastSeen

  constructor({ file, log = () => {}, now = () => Date.now(), onRevoke = () => {}, instanceName = () => null }) {
    this.store = new AuthStore(file);
    this.instanceName = instanceName; // affiché sur la page de connexion
    this.log = log;
    this.now = now;
    this.onRevoke = onRevoke; // ferme les flux SSE des sessions révoquées
  }

  get data() {
    return this.store.load();
  }

  // Protection active : au moins un compte et activée (ou fichier illisible : échec fermé).
  get enabled() {
    const d = this.data;
    return Boolean(this.store.error) || (d.enabled && d.users.length > 0);
  }

  audit(user, text, project = '-') {
    this.log(`[audit] ${project} ${user?.username ?? '-'} · ${text}`);
  }

  publicUser(u) {
    return { id: u.id, username: u.username, name: u.name || u.username, permissions: u.permissions, role: roleOf(u.permissions), prefs: u.prefs ?? {}, createdAt: u.createdAt, github: u.github ?? null };
  }

  setPrefs(userId, prefs) {
    const user = this.data.users.find((u) => u.id === userId);
    if (!user) return;
    user.prefs = { ...user.prefs, ...prefs };
    this.store.save();
  }

  // ---- Sessions ----

  #expiresAt(s) {
    return s.lastSeen + (s.remember ? SESSION_LONG : SESSION_SHORT);
  }

  // Session de la requête, ou null. Met à jour « dernière activité » (écrite au plus une fois par minute).
  session(req) {
    const token = parseCookies(req)[COOKIE];
    if (!token) return null;
    const d = this.data;
    const s = d.sessions.find((x) => x.id === sha256(token));
    if (!s) return null;
    const user = d.users.find((u) => u.id === s.userId);
    const now = this.now();
    if (!user || this.#expiresAt(s) < now) return null;
    s.lastSeen = now;
    if (now - (this.#seenSaved.get(s.id) ?? 0) > 60_000) {
      this.#seenSaved.set(s.id, now);
      this.store.save();
    }
    return { session: s, user };
  }

  isLocked(s) {
    const lockAfter = this.data.lockAfter;
    return Boolean(s.locked) || (lockAfter > 0 && this.now() - (s.activeAt ?? s.createdAt) > lockAfter * 1000 + 15_000);
  }

  // Contrôle d'accès d'une route. Renvoie l'utilisateur (null : protection désactivée).
  require(req, permission) {
    if (!this.enabled) return null;
    if (this.store.error) throw new AuthError(503, `${this.store.error}. Corrigez ou supprimez ce fichier.`, { auth: 'broken' });
    const ctx = this.session(req);
    if (!ctx) throw new AuthError(401, 'Connexion requise.', { auth: 'login' });
    if (this.isLocked(ctx.session)) throw new AuthError(401, 'Session verrouillée.', { auth: 'locked' });
    if (permission && !ctx.user.permissions.includes(permission) && !ctx.user.permissions.includes('admin')) {
      throw new AuthError(403, 'Votre compte n’a pas la permission de faire ceci.');
    }
    return ctx.user;
  }

  can(user, permission) {
    return !user || user.permissions.includes('admin') || user.permissions.includes(permission);
  }

  // Mot de passe redemandé avant une action grave (supprimer un projet), sous la même limite d'essais.
  async confirmPassword(req, user, password) {
    const account = this.data.users.find((u) => u.id === user.id);
    const ok = await this.#attempt(req, 'login', user.username, () => verifySecret(String(password ?? ''), account?.password));
    if (!ok) throw new AuthError(403, 'Mot de passe incorrect.');
  }

  #openSession(res, req, user, remember) {
    const d = this.data;
    const token = randomToken();
    const now = this.now();
    d.sessions.push({ id: sha256(token), userId: user.id, createdAt: now, lastSeen: now, activeAt: now, remember: Boolean(remember), agent: describeAgent(req.headers['user-agent']), ip: clientIp(req) });
    // Au plus MAX_SESSIONS par compte : les plus anciennes partent.
    const mine = d.sessions.filter((s) => s.userId === user.id).sort((a, b) => a.lastSeen - b.lastSeen);
    const drop = new Set(mine.slice(0, Math.max(0, mine.length - MAX_SESSIONS)).map((s) => s.id));
    d.sessions = d.sessions.filter((s) => !drop.has(s.id) && this.#expiresAt(s) >= now);
    this.store.save();
    const secure = req.socket?.encrypted ? '; Secure' : '';
    const age = remember ? `; Max-Age=${SESSION_LONG / 1000}` : '';
    res.setHeader('Set-Cookie', `${COOKIE}=${token}; HttpOnly; SameSite=Strict; Path=/${age}${secure}`);
  }

  #clearCookie(res) {
    res.setHeader('Set-Cookie', `${COOKIE}=; HttpOnly; SameSite=Strict; Path=/; Max-Age=0`);
  }

  // Ferme des sessions (toutes celles d'un compte, sauf éventuellement une).
  #revoke(filter) {
    const d = this.data;
    const gone = d.sessions.filter(filter).map((s) => s.id);
    d.sessions = d.sessions.filter((s) => !gone.includes(s.id));
    this.store.save();
    if (gone.length) this.onRevoke(gone);
    return gone.length;
  }

  // ---- Limite des essais ----

  #checkRate(keys) {
    const now = this.now();
    for (const key of keys) {
      const f = this.#failures.get(key);
      if (f?.until > now) {
        const s = Math.ceil((f.until - now) / 1000);
        throw new AuthError(429, `Trop d’essais : réessayez dans ${s < 60 ? `${s} s` : `${Math.ceil(s / 60)} min`}.`);
      }
    }
  }

  #fail(keys) {
    for (const [key, threshold] of keys) {
      const f = this.#failures.get(key) ?? { count: 0, until: 0 };
      f.count++;
      if (f.count >= threshold) f.until = this.now() + Math.min(30_000 * 2 ** (f.count - threshold), 15 * 60_000);
      this.#failures.set(key, f);
    }
  }

  #succeed(keys) {
    for (const key of keys) this.#failures.delete(key);
  }

  // Vérifie un secret sous limite d'essais (5 par compte et par adresse, 20 par adresse).
  async #attempt(req, kind, username, check) {
    const ip = clientIp(req);
    const keys = [`${kind}:${ip}:${username}`, `ip:${ip}`];
    this.#checkRate(keys);
    const ok = await check();
    if (!ok) {
      this.#fail([[keys[0], 5], [keys[1], 20]]);
      return false;
    }
    this.#succeed([keys[0]]);
    return true;
  }

  // ---- Comptes ----

  async #createUser({ username, name, password, permissions }) {
    const d = this.data;
    username = String(username ?? '').trim().toLowerCase();
    checkUsername(username);
    checkName(name);
    checkPassword(password);
    if (d.users.some((u) => u.username === username)) throw new AuthError(409, 'Ce nom d’utilisateur est déjà pris.');
    const recovery = humanCode(5);
    const user = {
      id: `u_${crypto.randomBytes(6).toString('hex')}`,
      username,
      name: name?.trim() || username,
      password: await hashSecret(password),
      recovery: await hashSecret(normalizeCode(recovery)),
      permissions: normalizePermissions(permissions),
      prefs: {},
      createdAt: this.now(),
    };
    d.users.push(user);
    return { user, recovery };
  }

  #admins() {
    return this.data.users.filter((u) => u.permissions.includes('admin'));
  }

  // ---- Routes /api/auth/... ----

  async handle(req, res, parts, { readJson, send }) {
    const [section, id, sub] = parts;
    const { method } = req;
    const body = method === 'GET' || method === 'DELETE' || !req.headers['content-type'] ? {} : await readJson(req);
    const d = this.data;
    const now = this.now();

    // -- Publiques --
    if (section === 'status' && method === 'GET') {
      const ctx = this.enabled && !this.store.error ? this.session(req) : null;
      return send(res, 200, {
        enabled: this.enabled,
        name: this.instanceName(),
        error: this.store.error,
        hasAccounts: d.users.length > 0,
        lockAfter: d.lockAfter,
        methods: { password: true },
        user: ctx ? this.publicUser(ctx.user) : null,
        locked: ctx ? this.isLocked(ctx.session) : false,
      });
    }
    if (this.store.error) throw new AuthError(503, `${this.store.error}. Corrigez ou supprimez ce fichier.`);

    if (section === 'setup' && method === 'POST') {
      // Première activation : crée le compte administrateur.
      if (d.users.length) throw new AuthError(409, 'Des comptes existent déjà : utilisez « Réactiver ».');
      const { user, recovery } = await this.#createUser({ ...body, permissions: ['admin'] });
      d.enabled = true;
      this.#openSession(res, req, user, body.remember);
      this.audit(user, 'Protection par mot de passe activée');
      return send(res, 201, { user: this.publicUser(user), recovery });
    }
    if (section === 'enable' && method === 'POST') {
      // Réactivation : il faut les identifiants d'un administrateur existant.
      if (this.enabled) throw new AuthError(409, 'La protection est déjà active.');
      const user = this.#admins().find((u) => u.username === String(body.username ?? '').toLowerCase());
      const ok = await this.#attempt(req, 'login', body.username, () => verifySecret(body.password ?? '', user?.password));
      if (!ok || !user) throw new AuthError(401, 'Identifiants d’administrateur incorrects.');
      d.enabled = true;
      this.#openSession(res, req, user, body.remember);
      this.audit(user, 'Protection réactivée');
      return send(res, 200, { user: this.publicUser(user) });
    }
    if (section === 'login' && method === 'POST') {
      if (!this.enabled) throw new AuthError(409, 'La protection n’est pas activée.');
      const username = String(body.username ?? '').trim().toLowerCase();
      const user = d.users.find((u) => u.username === username);
      const ok = await this.#attempt(req, 'login', username, () => verifySecret(body.password ?? '', user?.password));
      if (!ok || !user) {
        this.audit({ username: username || '?' }, `Échec de connexion depuis ${clientIp(req)}`);
        throw new AuthError(401, 'Nom d’utilisateur ou mot de passe incorrect.');
      }
      this.#openSession(res, req, user, body.remember);
      this.audit(user, 'Connexion');
      return send(res, 200, { user: this.publicUser(user) });
    }
    if (section === 'logout' && method === 'POST') {
      const ctx = this.session(req);
      if (ctx) {
        this.#revoke((s) => s.id === ctx.session.id);
        this.audit(ctx.user, 'Déconnexion');
      }
      this.#clearCookie(res);
      return send(res, 200, { ok: true });
    }
    if ((section === 'recover' || section === 'reset') && method === 'POST') {
      // recover : clé de secours · reset : code de 15 minutes (npm run auth ou un administrateur).
      const username = String(body.username ?? '').trim().toLowerCase();
      const user = d.users.find((u) => u.username === username);
      const secret = normalizeCode(section === 'recover' ? body.key : body.code);
      const ok = await this.#attempt(req, section, username, async () => {
        if (section === 'recover') return verifySecret(secret, user?.recovery);
        const r = d.resets.find((x) => x.userId === user?.id && x.expiresAt > now);
        return Boolean(r) && crypto.timingSafeEqual(Buffer.from(sha256(secret)), Buffer.from(r.codeHash));
      });
      if (!ok || !user) throw new AuthError(401, section === 'recover' ? 'Clé de secours incorrecte.' : 'Code incorrect ou expiré.');
      checkPassword(body.password);
      user.password = await hashSecret(body.password);
      d.resets = d.resets.filter((r) => r.userId !== user.id);
      // La clé de secours sert une fois : une nouvelle est générée.
      let recovery = null;
      if (section === 'recover') {
        recovery = humanCode(5);
        user.recovery = await hashSecret(normalizeCode(recovery));
      }
      this.#revoke((s) => s.userId === user.id);
      this.#openSession(res, req, user, false);
      this.audit(user, section === 'recover' ? 'Mot de passe réinitialisé avec la clé de secours' : 'Mot de passe réinitialisé avec un code');
      return send(res, 200, { user: this.publicUser(user), recovery });
    }
    if (section === 'invite') {
      const invite = d.invites.find((i) => i.tokenHash === sha256(method === 'GET' ? id ?? '' : body.token ?? '') && i.expiresAt > now);
      if (method === 'GET') {
        return send(res, 200, invite ? { valid: true, role: roleOf(invite.permissions), expiresAt: invite.expiresAt } : { valid: false });
      }
      if (method === 'POST') {
        if (!invite) throw new AuthError(410, 'Invitation invalide ou expirée.');
        const { user, recovery } = await this.#createUser({ ...body, permissions: invite.permissions });
        d.invites = d.invites.filter((i) => i !== invite);
        this.#openSession(res, req, user, body.remember);
        this.audit(user, `Compte créé sur invitation (${roleName(user.permissions)})`);
        return send(res, 201, { user: this.publicUser(user), recovery });
      }
    }

    // -- Session ouverte (même verrouillée pour déverrouiller) --
    if (!this.enabled) throw new AuthError(409, 'La protection n’est pas activée.');
    const ctx = this.session(req);
    if (!ctx) throw new AuthError(401, 'Connexion requise.', { auth: 'login' });
    const me = ctx.user;
    if (section === 'unlock' && method === 'POST') {
      const ok = await this.#attempt(req, 'login', me.username, () => verifySecret(body.password ?? '', me.password));
      if (!ok) throw new AuthError(401, 'Mot de passe incorrect.');
      Object.assign(ctx.session, { locked: false, activeAt: now });
      this.store.save();
      return send(res, 200, { ok: true });
    }
    if (this.isLocked(ctx.session)) throw new AuthError(401, 'Session verrouillée.', { auth: 'locked' });
    if (section === 'activity' && method === 'POST') {
      ctx.session.activeAt = now;
      this.store.save();
      return send(res, 200, { ok: true });
    }
    if (section === 'lock' && method === 'POST') {
      ctx.session.locked = true;
      this.store.save();
      this.onRevoke([ctx.session.id]); // plus d'évènements vers une page verrouillée
      return send(res, 200, { ok: true });
    }
    if (section === 'me') {
      if (!id && method === 'PATCH') {
        checkName(body.name);
        if (body.name !== undefined) me.name = body.name.trim() || me.username;
        if (body.prefs !== undefined) {
          const p = body.prefs;
          const ok = p && typeof p === 'object' && JSON.stringify(p).length < 500;
          if (!ok) throw new AuthError(400, 'Préférences invalides.');
          me.prefs = { ...me.prefs, ...p };
        }
        this.store.save();
        return send(res, 200, { user: this.publicUser(me) });
      }
      if (id === 'password' && method === 'POST') {
        const ok = await this.#attempt(req, 'login', me.username, () => verifySecret(body.current ?? '', me.password));
        if (!ok) throw new AuthError(401, 'Mot de passe actuel incorrect.');
        checkPassword(body.password);
        me.password = await hashSecret(body.password);
        const closed = this.#revoke((s) => s.userId === me.id && s.id !== ctx.session.id);
        this.audit(me, 'Mot de passe changé');
        return send(res, 200, { closed });
      }
      if (id === 'recovery' && method === 'POST') {
        const ok = await this.#attempt(req, 'login', me.username, () => verifySecret(body.password ?? '', me.password));
        if (!ok) throw new AuthError(401, 'Mot de passe incorrect.');
        const recovery = humanCode(5);
        me.recovery = await hashSecret(normalizeCode(recovery));
        this.store.save();
        this.audit(me, 'Nouvelle clé de secours générée');
        return send(res, 200, { recovery });
      }
      if (id === 'sessions') {
        if (method === 'GET') {
          const list = d.sessions.filter((s) => s.userId === me.id && this.#expiresAt(s) >= now)
            .sort((a, b) => b.lastSeen - a.lastSeen)
            .map((s) => ({ id: s.id.slice(0, 16), agent: s.agent, ip: s.ip, createdAt: s.createdAt, lastSeen: s.lastSeen, remember: s.remember, current: s.id === ctx.session.id }));
          return send(res, 200, { sessions: list });
        }
        if (method === 'DELETE' && sub) {
          const n = this.#revoke((s) => s.userId === me.id && s.id.startsWith(sub) && s.id !== ctx.session.id);
          return send(res, 200, { closed: n });
        }
        if (method === 'POST' && sub === 'others') {
          const n = this.#revoke((s) => s.userId === me.id && s.id !== ctx.session.id);
          this.audit(me, 'Autres sessions fermées');
          return send(res, 200, { closed: n });
        }
      }
    }

    // -- Administration --
    if (!me.permissions.includes('admin')) throw new AuthError(403, 'Réservé aux administrateurs.');
    if (section === 'users') {
      if (!id && method === 'GET') return send(res, 200, { users: d.users.map((u) => this.publicUser(u)) });
      const user = d.users.find((u) => u.id === id);
      if (!user) throw new AuthError(404, 'Compte inconnu.');
      const lastAdmin = user.permissions.includes('admin') && this.#admins().length === 1;
      if (!sub && method === 'PATCH') {
        checkName(body.name);
        if (body.permissions !== undefined) {
          const perms = normalizePermissions(body.permissions);
          if (lastAdmin && !perms.includes('admin')) throw new AuthError(409, 'Impossible de retirer les droits du dernier administrateur.');
          user.permissions = perms;
        }
        if (body.name !== undefined) user.name = body.name.trim() || user.username;
        this.store.save();
        this.audit(me, `Compte ${user.username} modifié (${roleName(user.permissions)})`);
        return send(res, 200, { user: this.publicUser(user) });
      }
      if (!sub && method === 'DELETE') {
        if (lastAdmin) throw new AuthError(409, 'Impossible de supprimer le dernier administrateur.');
        d.users = d.users.filter((u) => u !== user);
        d.resets = d.resets.filter((r) => r.userId !== user.id);
        this.#revoke((s) => s.userId === user.id);
        this.audit(me, `Compte ${user.username} supprimé`);
        return send(res, 200, { ok: true });
      }
      if (sub === 'reset' && method === 'POST') {
        const code = createResetCode(this.store, user.username, now);
        this.audit(me, `Code de réinitialisation créé pour ${user.username}`);
        return send(res, 200, { code, expiresAt: now + RESET_TTL });
      }
    }
    if (section === 'invites') {
      d.invites = d.invites.filter((i) => i.expiresAt > now);
      if (method === 'GET') {
        return send(res, 200, { invites: d.invites.map((i) => ({ id: i.tokenHash.slice(0, 16), role: roleOf(i.permissions), permissions: i.permissions, expiresAt: i.expiresAt, createdBy: i.createdBy })) });
      }
      if (method === 'POST') {
        const token = randomToken();
        const permissions = normalizePermissions(body.permissions ?? ROLES.operator);
        d.invites.push({ tokenHash: sha256(token), permissions, expiresAt: now + INVITE_TTL, createdBy: me.username });
        this.store.save();
        this.audit(me, `Invitation créée (${roleName(permissions)})`);
        return send(res, 201, { token, expiresAt: now + INVITE_TTL });
      }
      if (method === 'DELETE' && id) {
        d.invites = d.invites.filter((i) => !i.tokenHash.startsWith(id));
        this.store.save();
        return send(res, 200, { ok: true });
      }
    }
    if (section === 'settings' && method === 'PATCH') {
      if (body.lockAfter !== undefined) {
        if (!LOCK_CHOICES.includes(body.lockAfter)) throw new AuthError(400, 'Durée de verrouillage invalide.');
        d.lockAfter = body.lockAfter;
        for (const s of d.sessions) s.activeAt = now;
      }
      this.store.save();
      this.audit(me, `Verrouillage après inactivité : ${d.lockAfter ? `${d.lockAfter / 60} min` : 'jamais'}`);
      return send(res, 200, { lockAfter: d.lockAfter });
    }
    if (section === 'disable' && method === 'POST') {
      const ok = await this.#attempt(req, 'login', me.username, () => verifySecret(body.password ?? '', me.password));
      if (!ok) throw new AuthError(401, 'Mot de passe incorrect.');
      d.enabled = false;
      this.#revoke(() => true);
      this.#clearCookie(res);
      this.audit(me, 'Protection désactivée (les comptes sont conservés)');
      return send(res, 200, { ok: true });
    }
    throw new AuthError(404, 'Route inconnue');
  }
}
