// Adresse d'un projet selon qui ouvre la page : devant la machine, telle quelle ;
// depuis un autre appareil, « localhost » désigne la machine de Servym si le
// projet est exposé, et rien du tout s'il n'écoute que sur elle. Sans DOM :
// testé sous Node (tests/urls.test.js).
const LOOPBACK = new Set(['localhost', '127.0.0.1', '[::1]']);

// viewer : { local : page ouverte sur la machine de Servym, hostname : son nom vu d'ici }.
export function projectUrl(p, viewer) {
  let url;
  try {
    url = new URL(p.url);
  } catch {
    return null;
  }
  if (!['http:', 'https:'].includes(url.protocol)) return null;
  if (viewer.local || !LOOPBACK.has(url.hostname)) return url.href;
  if ((p.activeHost ?? p.host) !== '0.0.0.0') return null;
  url.hostname = viewer.hostname;
  return url.href;
}
