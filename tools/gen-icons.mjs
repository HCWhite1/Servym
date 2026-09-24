// Icônes de l'application de bureau, dessinées d'après public/favicon.svg (carré sombre arrondi,
// orbite, noyau et satellite), sans dépendance : rendu par suréchantillonnage, PNG et ICO écrits à
// la main. Variante « alerte » pour la zone de notification : satellite rouge (un projet en échec).
//
//   node tools/gen-icons.mjs
import { mkdirSync, writeFileSync } from 'node:fs';
import { deflateSync } from 'node:zlib';

const OUT = new URL('../crates/servynex-desktop/icons/', import.meta.url);
const DARK = [0x18, 0x18, 0x1b];
const WHITE = [0xff, 0xff, 0xff];
const RED = [0xef, 0x44, 0x44];

// Formes dans le repère 32×32 du SVG, peintes dans l'ordre : [test(x, y), couleur].
function shapes(satellite) {
  const inRoundRect = (x, y) => {
    const r = 8;
    const cx = Math.min(Math.max(x, r), 32 - r);
    const cy = Math.min(Math.max(y, r), 32 - r);
    return x >= 0 && x <= 32 && y >= 0 && y <= 32 && (x - cx) ** 2 + (y - cy) ** 2 <= r * r;
  };
  const dist = (x, y, cx, cy) => Math.hypot(x - cx, y - cy);
  return [
    [inRoundRect, DARK],
    [(x, y) => Math.abs(dist(x, y, 16, 16) - 7.5) <= 1.75 / 2, WHITE],
    [(x, y) => dist(x, y, 16, 16) <= 2.4, WHITE],
    // Satellite : contour sombre de 1,5 (moitié à l'extérieur), puis disque.
    [(x, y) => dist(x, y, 21.75, 11.18) <= 2.2 + 0.75, DARK],
    [(x, y) => dist(x, y, 21.75, 11.18) <= 2.2 - 0.75, satellite],
  ];
}

function render(size, satellite) {
  const S = 4; // 4×4 échantillons par pixel
  const list = shapes(satellite);
  const px = Buffer.alloc(size * size * 4);
  for (let py = 0; py < size; py++) {
    for (let pxl = 0; pxl < size; pxl++) {
      let r = 0, g = 0, b = 0, a = 0;
      for (let sy = 0; sy < S; sy++) {
        for (let sx = 0; sx < S; sx++) {
          const x = ((pxl + (sx + 0.5) / S) / size) * 32;
          const y = ((py + (sy + 0.5) / S) / size) * 32;
          let color = null;
          for (const [inside, c] of list) if (inside(x, y)) color = c;
          if (color) {
            r += color[0]; g += color[1]; b += color[2]; a += 1;
          }
        }
      }
      const i = (py * size + pxl) * 4;
      if (a) {
        px[i] = Math.round(r / a); px[i + 1] = Math.round(g / a); px[i + 2] = Math.round(b / a);
      }
      px[i + 3] = Math.round((a / (S * S)) * 255);
    }
  }
  return px;
}

const crcTable = Array.from({ length: 256 }, (_, n) => {
  let c = n;
  for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
  return c >>> 0;
});
const crc = (buf) => {
  let c = 0xffffffff;
  for (const byte of buf) c = crcTable[(c ^ byte) & 0xff] ^ (c >>> 8);
  return (c ^ 0xffffffff) >>> 0;
};
const chunk = (type, data) => {
  const len = Buffer.alloc(4);
  len.writeUInt32BE(data.length);
  const body = Buffer.concat([Buffer.from(type), data]);
  const sum = Buffer.alloc(4);
  sum.writeUInt32BE(crc(body));
  return Buffer.concat([len, body, sum]);
};

function png(size, satellite = WHITE) {
  const rgba = render(size, satellite);
  const ihdr = Buffer.alloc(13);
  ihdr.writeUInt32BE(size, 0);
  ihdr.writeUInt32BE(size, 4);
  ihdr[8] = 8; // 8 bits par canal
  ihdr[9] = 6; // RGBA
  const raw = Buffer.alloc((size * 4 + 1) * size);
  for (let y = 0; y < size; y++) rgba.copy(raw, y * (size * 4 + 1) + 1, y * size * 4, (y + 1) * size * 4);
  return Buffer.concat([
    Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]),
    chunk('IHDR', ihdr),
    chunk('IDAT', deflateSync(raw, { level: 9 })),
    chunk('IEND', Buffer.alloc(0)),
  ]);
}

// ICO : images PNG embarquées (acceptées depuis Windows Vista).
function ico(sizes) {
  const images = sizes.map((s) => png(s));
  const header = Buffer.alloc(6 + 16 * images.length);
  header.writeUInt16LE(1, 2);
  header.writeUInt16LE(images.length, 4);
  let offset = header.length;
  images.forEach((img, i) => {
    const e = 6 + 16 * i;
    header[e] = sizes[i] >= 256 ? 0 : sizes[i];
    header[e + 1] = sizes[i] >= 256 ? 0 : sizes[i];
    header.writeUInt16LE(1, e + 4); // plans
    header.writeUInt16LE(32, e + 6); // bits par pixel
    header.writeUInt32LE(img.length, e + 8);
    header.writeUInt32LE(offset, e + 12);
    offset += img.length;
  });
  return Buffer.concat([header, ...images]);
}

mkdirSync(OUT, { recursive: true });
writeFileSync(new URL('icon.ico', OUT), ico([16, 20, 24, 32, 40, 48, 64, 256]));
writeFileSync(new URL('32x32.png', OUT), png(32));
writeFileSync(new URL('128x128.png', OUT), png(128));
writeFileSync(new URL('128x128@2x.png', OUT), png(256));
writeFileSync(new URL('icon.png', OUT), png(512));
writeFileSync(new URL('tray.png', OUT), png(32));
writeFileSync(new URL('tray-alert.png', OUT), png(32, RED));
console.log('icônes écrites dans', OUT.pathname);
