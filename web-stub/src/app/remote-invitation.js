// Only public host identity is carried in an app invitation; pairing stays separate.
export async function inspectInvitation(value, cryptoProvider = globalThis.crypto) {
  if (typeof value !== 'string' || value.length > 66000) throw Error('invalid_invitation');
  const url = new URL(value);
  if (url.protocol !== 'tonalli:' || url.hostname !== 'library' || url.port || url.username || url.password || url.pathname || url.hash
    || [...url.searchParams].length !== 2 || url.searchParams.getAll('address').length !== 1 || url.searchParams.getAll('certificate').length !== 1) throw Error('invalid_invitation');
  const address = url.searchParams.get('address'), certificate = url.searchParams.get('certificate');
  if (!address || !certificate || certificate.length > 65536 || certificate.length % 2 || !/^[a-f0-9]+$/i.test(certificate)) throw Error('invalid_invitation');
  const bytes = Uint8Array.from(certificate.match(/../g), pair => parseInt(pair, 16));
  const digest = await cryptoProvider.subtle.digest('SHA-256', bytes);
  return { invitation:value, address, fingerprint: Array.from(new Uint8Array(digest), byte => byte.toString(16).padStart(2, '0').toUpperCase()).join(':') };
}
