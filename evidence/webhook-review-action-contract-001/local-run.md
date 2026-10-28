# CONTINUATION-122 — exiger l’action `reviewed` pour les avis PR Forgejo

Date : 26 octobre 2026. Dépôt produit au HEAD de base `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`; worktree très modifié, préservé. Aucun commit, reset, publication, changement de production, appel de modèle ou dépense.

## Constat et contrat amont

Le validateur commun `validate_event_payload_kind` contrôlait les trois paires en-tête / `review.type` (`pull_request_approved` / `pull_request_review_approved`, `pull_request_rejected` / `pull_request_review_rejected`, `pull_request_comment` / `pull_request_review_comment`) mais ne contrôlait pas `action`. Avant correction, un HMAC synthétique signé avec `action` absent était accepté en HTTP 202. Une ligne de journal équivalente avec action absente passait aussi la relecture et conduisait à un snapshot PR obtenu via le lecteur d’API.

La source primaire Forgejo officielle est épinglée à v17.0.0, commit `0d791421f85ca13a63b665053d81ea765fbbc571`, dans le scratch local `/tmp/forgejo-v17-base-cas-poc`. Les quatre fichiers consultés ont un arbre de travail propre par rapport à ce commit, même si le scratch global est modifié par des POC sans rapport. `services/webhook/notifier.go:727` fixe `Action: api.HookIssueReviewed`; `modules/structs/hook.go:315` définit cette valeur comme `reviewed`; `modules/webhook/type.go:73-77` mappe les trois types internes aux en-têtes externes. Le binding des noms d’en-tête et la signature du corps sont également documentés dans [FORGEJO-COMMENT-WIRE-001](../forgejo-comment-wire-001/local-run.md). Aucun code Forgejo n’est copié.

## Changement

Le validateur partagé exige désormais une chaîne égale à `reviewed` pour chacun de ces en-têtes. Une valeur absente, non textuelle ou différente retourne `InvalidEventPayloadField("action")`, qui donne un refus HTTP 400 actionnable lié à `payload.action`. Le même validateur est appelé à l’ingestion avant append et lors du parsing d’une ligne persistée avant tout appel à `ForgejoCurrentState`. La capacité découverte publie maintenant `action=reviewed` dans `required_payload_fields` et dans les contraintes pour les trois événements.

Aucun champ Cargo de licence n’a été changé. Le vérificateur actuel confirme `license=null`, `license_file=null`, `publish=[]`, métadonnée `no_license_selected`, droits `not_assessed` et marqueur d’export `NOASSERTION`; cela n’attribue aucune licence et ne statue pas sur les droits.

## TDD et vérification

Les tests ont été ajoutés avant le code. Les sorties rouges conservées montrent :

- `red-handler.log` : l’action absente est acceptée HTTP 202 au lieu d’être refusée avant journalisation.
- `red-reconcile.log` : la ligne sans action produit `Ok(ReconciledState{...})` au lieu d’être refusée avant le readback.

Après correction, les commandes ciblées passent :

- `webhook_handler_contract` : 27/27. Le nouveau test couvre trois en-têtes × trois formes invalides (action absente, chaîne `edited`, nombre 42), vérifie HTTP 400, `event_payload_invalid`, `payload.action` et inbox vide. Les tests existants gardent les formes valides acceptées.
- `webhook_reconcile_contract` : 56/56. Le nouveau replay teste action absente et `edited`, exige `InvalidEventPayloadField("action")` et zéro requête au lecteur.
- `webhook_reconcile_pipeline` : 3/3.
- Vérification du contrat découvrable incluse : le champ requis est publié pour les trois événements.
- Garde de licence Cargo : 9/9; JSON de `cargo metadata` et résultat de la garde conservés.
- `cargo fmt --all -- --check` et `git diff --check` : code de sortie 0.

Commandes exactes, versions, hashes source, sorties complètes et validation append-only : dans ce dossier. Le rapport ne compte pas les deux tests rouges comme succès.

## Reproduction

Depuis la racine du worktree :

```sh
cargo test --offline --locked --test webhook_handler_contract --test webhook_reconcile_contract --test webhook_reconcile_pipeline -- --test-threads=1
python3 scripts/verify_cargo_license_state.py
python3 -m unittest scripts.test_cargo_license_state
cargo metadata --locked --offline --format-version 1 --no-deps
cargo fmt --all -- --check
git diff --check
```

Pour reproduire les tests rouges depuis une copie exacte des sources avant le correctif, appliquer les deux commandes ciblées décrites dans `commands.txt`; elles sont conservées comme observations historiques et ne doivent pas être relancées sur les sources vertes.

## Limites

Les cas négatifs utilisent des corps signés par le secret de fixture et un lecteur API en mémoire; ils n’exercent pas le processus receiver complet ni une livraison émise par Forgejo. L’audit source du producteur est une observation de code v17 et ne prouve pas le comportement de chaque version/binaire. L’avis courant n’est pas une preuve d’autorisation de fusion, d’identité fiable du reviewer ou de qualité du code. Aucun LLM, AgentOS, RAG, mémoire, CI conventionnelle, mesure de tokens/coût, benchmark apparié ou gain n’est démontré.

## Validation des manifestes

Le premier `sha256sum -c SHA256SUMS` a passé 19/19 fichiers. Le manifeste append-only final couvre 20 noms; `verify_append_only_indexes.py` vérifie les dernières empreintes de ces noms, les index du worktree et du projet, leurs préfixes préexistants et le lien du parent vers l’index courant. `manifest-check.txt` et `global-manifest-check.txt` conservent le résultat. Les anciennes empreintes des versions antérieures de `local-run.md` et `source-sha256.txt` sont conservées append-only; la vérification finale se fait par la dernière entrée de chaque chemin, pas par toutes les lignes historiques de `sha256sum -c`.
