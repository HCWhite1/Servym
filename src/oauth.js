// Connexion par un fournisseur externe (OAuth 2, « web application flow »).
// Chaque installation déclare sa propre application chez le fournisseur ;
// l'identifiant et le secret sont gardés dans runtime/auth.json (src/auth.js).
// Un autre fournisseur s'ajoute ici : adresses, portée demandée, lecture du profil.
export const PROVIDERS = {
  github: {
    name: 'GitHub',
    authorizeUrl: 'https://github.com/login/oauth/authorize',
    tokenUrl: 'https://github.com/login/oauth/access_token',
    userUrl: 'https://api.github.com/user',
    scope: '', // profil public : identifiant et nom d'utilisateur, rien d'autre
    profile: (u) => ({ id: u.id === undefined ? '' : String(u.id), login: String(u.login ?? '') }),
    // Page où l'administrateur crée l'application (Paramètres › Sécurité).
    registerUrl: 'https://github.com/settings/applications/new',
  },
};

// overrides : { github: { tokenUrl, … } } (tests : faux fournisseur local).
export function providers(overrides = {}) {
  return Object.fromEntries(Object.entries(PROVIDERS).map(([key, p]) => [key, { ...p, ...overrides[key] }]));
}

export function authorizeUrl(provider, { clientId, redirectUri, state }) {
  const url = new URL(provider.authorizeUrl);
  url.search = new URLSearchParams({ client_id: clientId, redirect_uri: redirectUri, state, ...(provider.scope ? { scope: provider.scope } : {}), allow_signup: 'false' });
  return url.href;
}

// Code reçu au retour → profil { id, login } du compte chez le fournisseur.
export async function fetchProfile(provider, { clientId, clientSecret, code, redirectUri }) {
  const signal = AbortSignal.timeout(10_000);
  const headers = { Accept: 'application/json', 'User-Agent': 'Servynex' };
  const tokenRes = await fetch(provider.tokenUrl, {
    method: 'POST',
    headers: { ...headers, 'Content-Type': 'application/x-www-form-urlencoded' },
    body: new URLSearchParams({ client_id: clientId, client_secret: clientSecret, code, redirect_uri: redirectUri }),
    signal,
  });
  const token = (await tokenRes.json().catch(() => ({}))).access_token;
  if (!tokenRes.ok || typeof token !== 'string' || !token) throw new Error('token');
  const userRes = await fetch(provider.userUrl, { headers: { ...headers, Authorization: `Bearer ${token}` }, signal });
  if (!userRes.ok) throw new Error('profile');
  const profile = provider.profile(await userRes.json());
  if (!profile.id || !profile.login) throw new Error('profile');
  return profile;
}
