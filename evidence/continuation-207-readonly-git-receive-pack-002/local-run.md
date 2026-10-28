# CONTINUATION-207 — refus du Git receive-pack avec le credential read-only

## Objet et résultat

Le runner `scripts/run_forgejo_live_org_team_reviewer_readback.py` accepte maintenant l’option explicite `--probe-read-only-git-write`. Dans l’instance Forgejo jetable, le credential du reviewer peut lire la tête par Git HTTP (`git ls-remote` voit `refs/heads/main`) mais une tentative `git push` d’une nouvelle branche échoue (code 128). Forgejo écrit dans son access log un refus 403 sur la requête `GET /forge-team-org/team-review-readback.git/info/refs?service=git-receive-pack`. L’API admin relit les branches avant et après; `agent/read-only-git-write-probe` est absente dans les deux états. La sortie Git ne contient ni le token brut ni son Basic credential encodé.

La portée est précisément le refus de l’annonce `GET ...?service=git-receive-pack`. Le push n’a pas atteint un `POST /git-receive-pack`; ce test ne prétend donc pas démontrer un refus direct de chaque forme de requête Git write. Le même token reçoit aussi HTTP 403 sur le `POST /repos/forge-team-org/team-review-readback/issues/1/comments`; les commentaires et l’empreinte d’état de la PR sont inchangés. Ce sont deux refus côté autorisation Forgejo sur une instance/version, un compte, un dépôt et ces requêtes ciblées.

## Parcours conservé

Commande live exacte :

```sh
python3 scripts/run_forgejo_live_org_team_reviewer_readback.py \
  --forgejo-binary /tmp/forgejo-17.0.0 \
  --state-dir /tmp/forgejo-team-review-cont207-state-002 \
  --evidence-dir evidence/continuation-207-readonly-git-receive-pack-002 \
  --probe-read-only-git-write
```

Le run complet rapporte `verdict=passed`. Forgejo est `17.0.0+gitea-1.22.0`, compilé avec Go 1.26.9; SHA-256 du binaire `e5ec437cd6357e8d1b4580cb47ab1daa58a32b33971a63db63446a535d33adb7`. Le commit de build n’est pas connu. Le dépôt et l’organisation de fixture sont privés et jetables. Le compte reviewer a les scopes `read:user`, `read:repository`, `read:issue` et appartient à l’équipe `reviewers`.

Le webhook `pull_request/review_requested` est reçu en loopback; le CLI Rust, confiné pour la compilation par Bubblewrap, effectue la réconciliation en lecture seule et rend l’équipe courante `[{"id":2,"name":"reviewers"}]`. Le journal reste inchangé pendant le readback. Le run n’effectue pas de merge. Forgejo et receiver se terminent code 0; le relevé de cleanup ne trouve aucun listener résiduel. L’état conservé se trouve dans `/tmp/forgejo-team-review-cont207-state-002` (mode 0700); la configuration privée générée utilise `MODE=console`, `LEVEL=Info` pour rendre observable l’access log du test.

## Vérifications fraîches

- `python3 -m unittest discover -s tests -p 'test_forgejo_live_org_team_reviewer_readback.py' -v`: 11 tests, 0 échec.
- `python3 -m py_compile scripts/run_forgejo_live_org_team_reviewer_readback.py tests/test_forgejo_live_org_team_reviewer_readback.py`: code 0.
- `python3 scripts/verify_cargo_license_state.py`: `metadata_policy_verified`.
- `python3 -m unittest -v scripts.test_cargo_license_state`: 12 tests, 0 échec.
- `cargo metadata --locked --offline --format-version 1 --no-deps`: code 0; pour `agent-forge-core` 0.1.0, `license=null`, `license_file=null`, `publish=[]`, `no_license_selected`, droits `not_assessed`, marqueur `NOASSERTION`.
- `git diff --check`: code 0.

Les versions, commandes, reçus JSON expurgés, empreintes et manifestes sont dans ce répertoire. Aucun modèle/LLM/AgentOS n’a été appelé; aucune dépense externe ni donnée privée n’a été envoyée.

## Limites

Un seul binaire Forgejo local/version/configuration, utilisateur, dépôt et tentative (`n=1`). L’access log est la source du statut 403; les readbacks de branches confirment l’absence de l’effet observé. Aucun capture réseau indépendante, aucun `POST /git-receive-pack`, SSH, push d’un autre protocole, autre rôle/scope, autre route writer ou test de déploiement n’est couvert. Cette preuve d’ACL Forgejo ne teste pas l’isolation de processus ni un agent réel; elle ne garantit pas un contrôle exhaustif de toutes les routes. L’intégration complète, l’autorité de merge universelle, la comparaison appariée et les autres critères de mission restent ouverts.
