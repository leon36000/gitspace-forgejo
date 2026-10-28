# FORGEJO-PRIVATE-RECONCILE-BOUNDARY-001

Date : 25 octobre 2026. Exécution locale isolée dans le worktree `local/WEBHOOK-005-canonical-links`, HEAD observé `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`. Aucun commit autonome n'est revendiqué.

## Question et résultat

Le lot ajoute un contrôle négatif à la réconciliation Forgejo déjà exercée sur un dépôt privé : l'API doit cacher le même dépôt sans authentification, puis le chemin autorisé doit permettre une lecture canonique par un credential hôte en lecture seule.

Le runner a lancé localement Forgejo `17.0.0+gitea-1.22.0` (`/tmp/forgejo-17.0.0`, SHA-256 `e5ec437cd6357e8d1b4580cb47ab1daa58a32b33971a63db63446a535d33adb7`). Son build exact depuis un commit Forgejo n'est pas établi. Il a créé `forge-admin/agent-forge-core` comme dépôt privé et confirmé le compte lecteur non-admin avec `pull=true`, `push=false`, `admin=false`.

Après le readback authentifié qui confirme la visibilité du dépôt privé et avant la lecture canonique de branche, le runner effectue un `GET /api/v1/repos/forge-admin/agent-forge-core` sans en-tête `Authorization`; Forgejo répond 404. Il crée deux vrais commits Git sur une branche privée, reçoit les deux événements `push` natifs via le hook local et les réconcilie par le binaire Rust. Le credential en lecture seule entre dans le processus par FD hérité privé, jamais dans argv ou l'environnement du CLI de réconciliation. Les deux résultats sont liés aux UUID demandés; une seule lecture upstream (`upstream_read_attempts=1`) renvoie la tête Forgejo courante pour l'événement ancien et le plus récent. Le journal append-only reste identique. Le runner vérifie que le credential n'apparaît pas dans stdout/stderr et n'enregistre aucune valeur secrète dans les reçus.

Le verdict est `passed`. Les hooks principal et secondaire ont été désactivés puis relus; Forgejo et le receiver ont terminé avec code 0, et leurs ports 3130 et 32967 sont absents du readback final. L'état privé sous `/tmp/forgejo-private-reconcile-boundary-001-state` est conservé.

## Test d'abord, puis vérifications

Le nouveau test ciblé a d'abord été exécuté avant l'implémentation; il a échoué avec trois cas `PrivateRepositoryAnonymousReadTests` indiquant que le validateur manquait. Après ajout de la validation stricte (`private is True` et statut HTTP exactement 404), les trois cas passent : preuve cohérente, refus si le dépôt privé répond 200, refus si la réponse Forgejo ne confirme pas `private=true`. Le transcript rouge a été observé dans la sortie directe mais n'a pas été sauvegardé comme fichier séparé.

Commandes et résultats frais :

```text
PYTHONDONTWRITEBYTECODE=1 python3 -m unittest discover -s tests -p 'test_forgejo_emitted_webhook_e2e.py' -k PrivateRepositoryAnonymousReadTests -v
  3 tests passed

PYTHONDONTWRITEBYTECODE=1 python3 scripts/run_forgejo_native_push_webhook_reconcile.py \
  --forgejo-binary /tmp/forgejo-17.0.0 \
  --state-dir /tmp/forgejo-private-reconcile-boundary-001-state \
  --evidence-dir evidence/forgejo-private-reconcile-boundary-001
  exit 0; verdict passed; anonymous API 404; authenticated batch current_state_read

PYTHONDONTWRITEBYTECODE=1 python3 -m unittest discover -s tests -v
  173 tests passed

cargo test --offline --locked --all-targets -- --test-threads=1
  511 passed, 0 failed, 61 ignored across 34 test groups

python3 scripts/verify_cargo_license_state.py
python3 scripts/test_cargo_license_state.py -v
cargo metadata --locked --offline --format-version 1
  verified; 7 license-state tests passed; license=null; license_file=null;
  publish=[]; decision=no_license_selected; redistribution_rights=not_assessed

cargo fmt --all -- --check
git diff --check
python3 -m py_compile scripts/run_forgejo_native_push_webhook_reconcile.py tests/test_forgejo_emitted_webhook_e2e.py
  all passed
```

Une vérification initiale du manifeste local et une lecture des ports ont été lancées avec des chemins relatifs depuis le mauvais répertoire; elles ont échoué uniquement à résoudre ces chemins. Les mêmes contrôles ont ensuite été exécutés depuis les répertoires corrects et ont passé. `sha256sum -c SHA256SUMS` valide le manifeste local après régénération. Les journaux et readbacks sont dans ce dossier.

Versions : Python 3.12.3, Rust/Cargo 1.95.0, Git 2.43.0, Bubblewrap 0.9.0; Forgejo annonce Go 1.26.9. Les empreintes de sources/exécutables et la commande de build offline sont dans `result.json`, `cargo-build.log`, `toolchain.txt` et `SHA256SUMS`.

## Portée et limites

Ce run prouve une frontière d'authentification sur une instance Forgejo locale et son adaptateur de réconciliation, pour un seul dépôt et un lot de deux événements. Il ne prouve ni l'isolation multi-tenant ni l'exclusion de writers sous le même UID; le binaire Forgejo n'a pas de provenance de build liée au commit audité. Aucun AgentOS, Forge Code, RAG, mémoire, LLM ou modèle n'a été invoqué; aucun coût de modèle ou gain comparatif n'est mesuré. Il ne s'agit pas d'un parcours de fusion. Le contrôle d'API anonyme et les événements sont locaux; aucun secret réel, service externe, production, publication ou changement Forgejo amont n'a été utilisé.

Cargo reste volontairement non publiable et sans choix de licence. Les droits de redistribution ne sont pas évalués; ce résultat ne donne aucun droit.
