import assert from 'node:assert/strict';
import { describe, test } from 'node:test';
import { projectUrl } from '../public/js/urls.js';

describe('adresse d’un projet selon qui ouvre la page', () => {
  const project = (url, host = '127.0.0.1') => ({ url, host, activeHost: null });
  const remote = { local: false, hostname: '192.168.1.5' };

  test('devant la machine : l’adresse telle quelle', () => {
    assert.equal(projectUrl(project('http://localhost:3000/x'), { local: true, hostname: 'localhost' }), 'http://localhost:3000/x');
  });

  test('depuis le réseau : un projet exposé s’ouvre par l’adresse de la machine', () => {
    assert.equal(projectUrl(project('http://localhost:3000/x?a=1', '0.0.0.0'), remote), 'http://192.168.1.5:3000/x?a=1');
    assert.equal(projectUrl({ url: 'http://127.0.0.1:3000/', host: '127.0.0.1', activeHost: '0.0.0.0' }, remote), 'http://192.168.1.5:3000/', 'adresse du lancement en cours');
  });

  test('depuis le réseau : un projet qui n’écoute que sur la machine est injoignable', () => {
    assert.equal(projectUrl(project('http://localhost:3000/'), remote), null);
  });

  test('adresse déjà explicite : gardée ; adresse dangereuse ou absente : aucune', () => {
    assert.equal(projectUrl(project('https://monsite.example/'), remote), 'https://monsite.example/');
    assert.equal(projectUrl(project('javascript:alert(1)'), { local: true, hostname: 'localhost' }), null);
    assert.equal(projectUrl(project(null), remote), null);
  });
});
