# CONTINUATION-210 — contrôle positif receive-pack avec le même pack Git

## Résultat

CONTINUATION-209 envoyait une commande de création de branche et un pack Git valides au credential read-only, mais Forgejo renvoyait 403 avant que le serveur ait à accepter le protocole. Ce run ajoute un contrôle positif dans une nouvelle instance privée et jetable : il envoie le même pack et le même commit sous la commande distincte `refs/heads/agent/read-write-control-probe`, avec le credential administrateur local du harnais (`write:repository`).

Forgejo 17.0.0 renvoie 403 à l’annonce `GET .../info/refs?service=git-receive-pack`, au POST flush vide et au POST portant la demande read-only et son pack. Le readback admin confirme l’absence de `agent/read-only-git-write-probe` avant et après. Le POST de contrôle sous credential write-enabled reçoit HTTP 200; le validateur observe `unpack ok` et le `report-status` positif pour la ref attendue. Le readback API retrouve `agent/read-write-control-probe` au commit `79222d6c2e20b4174f8e7010af0dd8cdf71ec663`, identique au nouvel OID envoyé. L’access log local lié à la route contient deux POST 403 et un POST 200. Le protocole autorisé et le contenu du pack sont donc acceptés sur cette instance; l’écart entre refus et succès correspond au credential utilisé.

Le pack fait 322 octets (`SHA-256 07b9619c8a3e86cae0d25c5439a575ac04c5d83067352e4693f159dbb73b0b11`); le paquet read-only fait 469 octets et le paquet de contrôle 468 octets, différence due aux longueurs des refs ciblées. Le reçu garde le statut et le readback; le corps HTTP brut n’est pas archivé. Les événements d’accès sont conservés sous forme filtrée, méthode/route/statut seulement, dans `receive-pack-access-events.json`; `verify_run_result.py` les recoupe avec le journal Forgejo et vérifie le readback, les refus, les arrêts et les permissions du dossier privé (`run-validation.json`).

## Run reproductible

```sh
PYTHONPYCACHEPREFIX=/tmp/forgejo-cont210-pycache python3 scripts/run_forgejo_live_org_team_reviewer_readback.py \
  --forgejo-binary /tmp/forgejo-17.0.0 \
  --state-dir /tmp/forgejo-team-review-cont210-state-001 \
  --evidence-dir evidence/continuation-210-receive-pack-positive-control-001 \
  --probe-read-only-git-write
```

Le run complet retourne `verdict=passed`, code de sortie 0. Il répète le setup de PR privée, l’événement `pull_request/review_requested` et la lecture de l’équipe non vide afin d’exécuter le probe dans le harnais intégré existant; cette partie n’est pas une nouvelle preuve du parcours agent. La PR reste ouverte/non fusionnée. Le compte read-only du probe a `read:user,read:repository,read:issue`; le credential du contrôle est un admin créé dans l’instance privée du test, avec `write:repository` parmi ses scopes. Aucun token n’est conservé dans les reçus.

Forgejo : `17.0.0+gitea-1.22.0`, Go 1.26.9, binaire SHA-256 `e5ec437cd6357e8d1b4580cb47ab1daa58a32b33971a63db63446a535d33adb7`; commit exact de build inconnu. Python 3.12.3; Rust/Cargo 1.95.0; Bubblewrap 0.9.0. Le harness compile les deux binaires Rust hors réseau avec Cargo offline/verrouillé, source montée en lecture seule et namespaces réseau/PID non partagés. L’instance Forgejo et son HTTP restent liés au loopback local. Le dossier d’état `/tmp/forgejo-team-review-cont210-state-001` est mode 0700 et préservé; le serveur, le receiver et les listeners sont arrêtés (codes de sortie 0).

## Vérifications

Le module Python du harnais passe 15/15, dont les tests du validator du contrôle positif et les refus pour statut absent/403, rapport manquant, branche ou OID incorrects, branche préexistante et route étrangère. `py_compile`, `scripts/verify_cargo_license_state.py`, ses 12 tests, `cargo metadata --locked --offline --format-version 1 --no-deps`, le contrôle d’espaces des fichiers ajoutés et `git diff --check` passent. Cargo continue d’exposer `license=null`, `license_file=null`, `publish=[]`, `no_license_selected`, `redistribution_rights=not_assessed` (`NOASSERTION`).

## Portée et limites

Cette exécution `n=1` prouve un cas d’autorisation read-only refusée et un contrôle positif write-enabled pour la même instance, le même dépôt privé, le même binaire/configuration et le même pack; les identités et rôles diffèrent (membre reviewer read-only contre administrateur). Elle répond au doute sur la validité de ce payload, mais n’isole pas les scopes du rôle, ni n’établit le parsing de toutes les variantes Git, les ACL de tous les rôles, routes API/Web/SSH, installations, les writers concurrents, une autorité merge exclusive ou la sécurité du processus d’un agent. L’access log est l’instrumentation de Forgejo local et le corps HTTP brut n’est pas conservé.

Aucun agent réel, LLM, AgentOS, benchmark, service externe, secret partagé, coût, publication, production, commit ou code Forgejo/GitSpace copié. Aucun écart comparatif ou gain de valeur n’est démontré. La mission demeure incomplète; les critères cumulatifs et limites restant ouverts sont suivis dans `docs/checkpoint.md` et `docs/refusal-evidence-matrix.md`.
