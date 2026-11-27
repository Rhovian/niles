import * as oauth from 'oauth4webapi';
import { z } from 'zod';

export const oauthSettings = z.object({ issuer: z.url({ protocol: /^https$/ }), client_id: z.string().trim().min(1), host_token_exchange: z.boolean().optional() });
export type OAuthSettings = z.infer<typeof oauthSettings>;
const transaction = oauthSettings.extend({ resource: z.url(), verifier: z.string(), state: z.string(), hash: z.string() });
const storedToken = z.object({ accessToken: z.string().min(1), refreshToken: z.string().min(1).optional() });
const bearerResponse = z.object({
  access_token: z.string().min(1), token_type: z.string().regex(/^bearer$/i, 'The issuer did not return a bearer token.'),
  refresh_token: z.string().min(1).optional(),
});

async function hostToken(response: Response) {
  if (!response.ok) throw new Error(`Host token exchange failed (HTTP ${response.status}).`);
  return bearerResponse.parse(await response.json());
}
type StoredToken = z.infer<typeof storedToken>;

function readToken(key: string): StoredToken | undefined {
  const saved = localStorage.getItem(key);
  if (saved !== null) return storedToken.parse(JSON.parse(saved));
}

function exchange(resource: string, grant: { code: string | null; code_verifier: string } | { refresh_token: string }) {
  return fetch(new URL(`${resource}/auth/token`).pathname, {
    method: 'POST', credentials: 'omit', cache: 'no-store', redirect: 'error',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(grant),
  });
}
const pending = 'niles.oauth.pending';
const callback = () => new URL('/', location.origin);
const tokenKey = (resource: string) => `niles.oauth.token:${resource}`;
export const hostResource = (prefix: string) => new URL(`${prefix}/api`, location.origin).href;

// oauth4webapi validates discovery, state and the token response, and performs the PKCE exchange.
// https://github.com/panva/oauth4webapi/tree/v3.8.6/docs
async function discover(issuer: string) {
  const url = new URL(issuer);
  return oauth.processDiscoveryResponse(url, await oauth.discoveryRequest(url));
}

export async function authorizationUrl(prefix: string, { issuer, client_id, host_token_exchange }: OAuthSettings) {
  const resource = hostResource(prefix);
  const server = await discover(issuer);
  if (!server.authorization_endpoint || new URL(server.authorization_endpoint).protocol !== 'https:') {
    throw new Error('The issuer has no HTTPS authorization endpoint.');
  }
  const verifier = oauth.generateRandomCodeVerifier();
  const state = oauth.generateRandomState();
  const url = new URL(server.authorization_endpoint);
  for (const [name, value] of Object.entries({
    client_id, response_type: 'code', redirect_uri: callback().href,
    resource, code_challenge: await oauth.calculatePKCECodeChallenge(verifier),
    code_challenge_method: 'S256', state,
  })) url.searchParams.set(name, value);
  sessionStorage.setItem(pending, JSON.stringify({ issuer, client_id, host_token_exchange, resource, verifier, state, hash: location.hash }));
  return url;
}

export async function finishSignIn() {
  const returned = new URL(location.href);
  if (!returned.searchParams.has('code') && !returned.searchParams.has('error')) return;
  history.replaceState(null, '', callback());
  const saved = sessionStorage.getItem(pending);
  sessionStorage.removeItem(pending);
  const flow = transaction.parse(JSON.parse(saved ?? 'null'));
  history.replaceState(null, '', `/${flow.hash}`);
  const server = await discover(flow.issuer);
  const client = { client_id: flow.client_id };
  const result = oauth.validateAuthResponse(server, client, returned.searchParams, flow.state);
  const response = flow.host_token_exchange
    ? await exchange(flow.resource, { code: result.get('code'), code_verifier: flow.verifier })
    : await oauth.authorizationCodeGrantRequest(server, client, oauth.None(), result,
      callback().href, flow.verifier, { additionalParameters: { resource: flow.resource } });
  const token = flow.host_token_exchange ? await hostToken(response)
    : bearerResponse.parse(await oauth.processAuthorizationCodeResponse(server, client, response));
  const key = tokenKey(flow.resource);
  await navigator.locks.request(key, () => localStorage.setItem(key, JSON.stringify({
    accessToken: token.access_token, refreshToken: flow.host_token_exchange ? token.refresh_token : undefined,
  })));
  return flow.resource;
}

export async function apiFetch(path: string | URL, options: RequestInit = {}) {
  const url = new URL(path, location.origin);
  const match = /^(\/h\/[A-Za-z0-9_-]+)?\/api\//.exec(url.pathname);
  if (url.origin !== location.origin || !match) throw new Error('Invalid Niles API URL.');
  const resource = hostResource(match[1] ?? '');
  const key = tokenKey(resource);
  const send = (token?: string) => {
    const headers = new Headers(options.headers);
    if (token !== undefined) headers.set('Authorization', `Bearer ${token}`);
    return fetch(url.pathname + url.search, {
      ...options, headers, credentials: 'omit', cache: 'no-store', redirect: 'error',
    });
  };
  const stored = readToken(key);
  const response = await send(stored?.accessToken);
  if (response.status !== 401 || !stored?.refreshToken) return response;

  // tsidp refresh tokens rotate on use. Web Locks serialize refreshes across tabs.
  // https://w3c.github.io/web-locks/
  const renewed = await navigator.locks.request(key, async () => {
    const current = readToken(key);
    if (current?.accessToken !== stored.accessToken) return current?.accessToken;
    if (!current.refreshToken) return;
    const result = await exchange(resource, { refresh_token: current.refreshToken });
    if (result.status === 400) {
      localStorage.setItem(key, JSON.stringify({ accessToken: current.accessToken }));
      return;
    }
    const token = await hostToken(result);
    localStorage.setItem(key, JSON.stringify({
      accessToken: token.access_token, refreshToken: token.refresh_token ?? current.refreshToken,
    }));
    return token.access_token;
  });
  return renewed === undefined ? response : send(renewed);
}
