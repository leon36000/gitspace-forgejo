# CONTINUATION-208 — refus direct du POST Git receive-pack par credential read-only

## Résultat observé

Le validateur précédent acceptait un 403 sur n’importe quelle ligne de log contenant `git-receive-pack`; son test isolé simulait un POST, mais le run live n’avait prouvé que le GET d’annonce. Les nouveaux tests exigent explicitement (1) `GET .../info/refs?service=git-receive-pack` à 403, (2) statut 403 retourné par le POST direct et (3) access log contenant `POST /forge-team-org/team-review-readback.git/git-receive-pack ... 403`. Ils refusent un POST réussi, une annonce seule à 403 ou un 403 sur un autre dépôt.

Le run final démarre une instance locale et privée Forgejo v17.0.0, utilise un token reviewer `read:user,read:repository,read:issue`, lit `refs/heads/main`, tente `git push`, puis envoie aussi un POST direct à l’URI `/forge-team-org/team-review-readback.git/git-receive-pack` via `http.client.HTTPConnection("127.0.0.1",3130)`. Aucun redirect handler n’est utilisé. Le corps `0000` est un pkt-line flush sans commande ni packfile; le test atteint et exerce la route d’autorisation, mais ne simule pas un push valide.

Forgejo retourne 403 aux deux méthodes Git : GET d’annonce et POST direct. `git push` termine code 128; les readbacks d’administration ne trouvent pas la branche `agent/read-only-git-write-probe` avant ni après les deux tentatives. Le token et son Basic credential ne figurent pas dans la sortie Git. Le `POST /repos/forge-team-org/team-review-readback/issues/1/comments` reçoit aussi 403; les commentaires et l’empreinte de la PR ne changent pas. Le webhook `pull_request/review_requested` est réconcilié par le CLI Rust, qui retourne `current_state_read` et `requested_reviewers_teams=[{"id":2,"name":"reviewers"}]`; aucun merge n’a lieu.

Run final exact :

```sh
python3 scripts/run_forgejo_live_org_team_reviewer_readback.py \
  --forgejo-binary /tmp/forgejo-17.0.0 \
  --state-dir /tmp/forgejo-team-review-cont208-state-002 \
  --evidence-dir evidence/continuation-208-readonly-git-receive-pack-post-002 \
  --probe-read-only-git-write
```

Il rapporte `verdict=passed`. Forgejo `17.0.0+gitea-1.22.0`, Go 1.26.9, binaire SHA-256 `e5ec437cd6357e8d1b4580cb47ab1daa58a32b33971a63db63446a535d33adb7`; commit de build inconnu. Build Rust isolé via Bubblewrap 0.9.0, `rustc 1.95.0`, Cargo 1.95.0; source read-only, réseau/PID non partagés, Cargo `--offline --locked`. Le cleanup indique les deux processus à code 0 et aucun listener résiduel. État fixture conservé en mode 0700 à `/tmp/forgejo-team-review-cont208-state-002`.

## TDD et vérifications

Le test-first a échoué avant le changement parce que `validate_read_only_git_receive_pack_probe` n’acceptait pas encore `direct_post_status` (deux tests lancés, neuf erreurs de sous-cas, code 1). Après la modification, les deux tests du validateur passent; le module `test_forgejo_live_org_team_reviewer_readback.py` passe 11/11. `py_compile` et `git diff --check` passent. Le garde de métadonnées Cargo doit rester vérifié avant fin de lot : aucune licence n’est choisie (`license=null`, `license_file=null`, `publish=[]`, `no_license_selected`, droits `not_assessed`, SPDX `NOASSERTION`).

Le run 001 dans le dossier voisin a aussi observé 403 sur POST via la version initiale du transport. Le run 002 est le reçu final après remplacement par une connexion loopback sans redirections. Aucun code Forgejo/GitSpace n’est copié; aucun modèle, dépense, service externe ou production n’est impliqué.

## Limites

Deux instances jetables locales, un binaire/version, un compte, un dépôt et les scopes listés (`n=2` runs ciblés, pas un échantillon indépendant). Le POST direct porte un pkt-line vide; il ne teste pas un packfile valide ni les autres chemins HTTP/SSH, roles ou configurations. Le test n’observe pas une capture réseau indépendante et ne prouve pas une politique exhaustive de toutes les routes d’écriture. Il vérifie l’autorisation Forgejo pour ces endpoints et ce token, pas une frontière de processus, un agent réel ou une autorité de merge générale. La mission et le benchmark comparatif restent incomplets.
