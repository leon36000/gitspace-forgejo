# PC4 GitLab reachability follow-up

Checked on 28 October 2026 after the user requested migration to `https://pc4.tail0c55ff.ts.net/root/gitspace-forgejo.git` and directed us to the access mail.

- `tailscale ping --c 3 pc4`: succeeds; the peer is reachable at the Tailscale layer.
- `tailscale dns status`: local `accept-dns` is disabled, although MagicDNS is enabled for the tailnet; the FQDN does not resolve through the local resolver.
- `git ls-remote https://pc4.tail0c55ff.ts.net/root/gitspace-forgejo.git`: fails before authentication with `Could not resolve host`.
- HTTPS probe with `curl --resolve` (hostname preserved for TLS): times out after 12 seconds.
- TCP probes from this host: ports 443, 80 and 22 are unreachable.

No credential was supplied to Git or curl. No remote refs, login, file, branch, commit or push occurred. The connected mail contains a token whose target repository scope was not established; the token is neither quoted nor saved here. A connector excerpt included its value in the tool response, so rotation is recommended. The initial administrator password was not used. Do not repeat the same network probes until the local DNS/service state changes or the user reports that it has changed.
