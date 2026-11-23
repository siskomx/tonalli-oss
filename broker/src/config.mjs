import { readFile, writeFile, rename, mkdir } from 'node:fs/promises';
import { randomBytes, scrypt as derive, timingSafeEqual } from 'node:crypto';
import { promisify } from 'node:util';
import path from 'node:path';
import os from 'node:os';
import {isIP} from 'node:net';
const scrypt = promisify(derive);
export const identifier = value => typeof value === 'string' && /^[a-zA-Z0-9_-]{1,64}$/.test(value);
export const privateAddress=value=>isIP(value)!==0&&/^(127\.0\.0\.1|10\.\d+\.\d+\.\d+|192\.168\.\d+\.\d+|172\.(1[6-9]|2\d|3[01])\.\d+\.\d+|::1|f[cd][0-9a-f:]+)$/i.test(value);
export function localNetworks(){return [{address:'127.0.0.1',name:'This PC'},...Object.entries(os.networkInterfaces()).flatMap(([name,entries])=>(entries||[]).filter(e=>e.family==='IPv4'&&!e.internal&&privateAddress(e.address)).map(e=>({address:e.address,name})))];}
export async function passwordHash(password) {
  const salt = randomBytes(16).toString('hex');
  const hash = await scrypt(password, salt, 32);
  return `${salt}:${hash.toString('hex')}`;
}
export async function verifyPassword(password, encoded) {
  if (typeof password !== 'string' || password.length > 256 || !/^[a-f0-9]{32}:[a-f0-9]{64}$/.test(encoded || '')) return false;
  const [salt, hash] = encoded.split(':');
  return timingSafeEqual(await scrypt(password, salt, 32), Buffer.from(hash, 'hex'));
}
export function validate(config) {
  if (config.version !== 1 || !['local','hosted'].includes(config.mode) || !Number.isInteger(config.port) || config.port < 1024 || config.port > 65535) throw Error('Invalid broker configuration');
  if (config.mode === 'local' && !(config.bind==='0.0.0.0' && process.env.TONALLI_CONTAINER==='1') && !privateAddress(config.bind)) throw Error('Local broker requires a specific private interface');
  if (!Array.isArray(config.rooms) || config.rooms.length > 64 || !config.rooms.length || !Array.isArray(config.users) || config.users.length > 128 || !config.users.length) throw Error('Invalid rooms or users');
  if (new Set(config.rooms.map(r=>r.id)).size !== config.rooms.length || new Set(config.users.map(u=>u.id)).size !== config.users.length) throw Error('Duplicate room or user');
  for (const r of config.rooms) if (!identifier(r.id) || typeof r.name !== 'string' || !r.name.trim() || r.name.length > 80) throw Error('Invalid room');
  if(new Set(config.rooms.map(r=>r.name.trim().toLowerCase())).size!==config.rooms.length)throw Error('Room names must be distinct');
  for (const u of config.users) {
    if (!identifier(u.id) || typeof u.name !== 'string' || !u.name.trim() || u.name.length > 80 || !/^[a-f0-9]{32}:[a-f0-9]{64}$/.test(u.passwordHash) || !['admin','participant','gateway'].includes(u.role)) throw Error('Invalid user');
    for (const permission of ['read','send','bridge']) if (!Array.isArray(u[permission]) || u[permission].length > 64 || u[permission].some(id=>!config.rooms.some(r=>r.id===id))) throw Error('Invalid room permission');
    if (u.role !== 'gateway' && u.bridge.length) throw Error('Only a gateway can bridge radio traffic');
  }
  if (!config.users.some(u=>u.role==='admin')) throw Error('Administrator required');
  return config;
}
export async function loadConfig(directory) { return validate(JSON.parse(await readFile(path.join(directory,'config.json'),'utf8'))); }
export async function saveConfig(directory, config) {
  validate(config);
  const staging = path.join(directory, `config-${randomBytes(8).toString('hex')}.tmp`);
  await writeFile(staging, JSON.stringify(config,null,2), {mode:0o600,flag:'wx'});
  await rename(staging,path.join(directory,'config.json'));
}
export async function privateDirectory(directory) { await mkdir(directory,{recursive:true,mode:0o700}); }
