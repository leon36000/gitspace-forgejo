# WEBHOOK-CLI-ADMISSION-001 — parcours processus vers claim

Date de vérification : 2026-10-23 01:20:42 UTC
Checkout : `local/WEBHOOK-005-canonical-links`, HEAD `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`
État : travail local non commité; le binaire et les contrats ajoutés dans les lots précédents sont des fichiers locaux non suivis par Git.

## Résultat

`forgejo-webhook-local` reçoit une requête synthétique signée, écrit l’événement dans son inbox privée et `forgejo-mission-local` retrouve cette livraison dans un processus séparé. Le scénario commence avec une issue fermée : le CLI retourne `inconclusive / mission_issue_closed` et ne crée aucun journal d’allocation. La fixture passe ensuite à `open`; le même CLI fait le claim. Un doublon du même webhook garde la séquence à 1; sa relance et une deuxième livraison avec un nouvel ID retournent `already_claimed` et conservent mission, génération 1 et allocation `admission-41`. Le scénario contrôle 31 lectures HTTP contre la fixture.

La fixture fournit les réponses Forgejo sur `127.0.0.1:3130`, avec un dépôt, une exigence JSON v2 liée à l’issue #41, un commit et deux blobs synthétiques. Le receiver, le CLI de mission et la lecture des deux journaux sont réels; aucune instance Forgejo n’est appelée. La clé HMAC est synthétique, créée en `0600` dans un répertoire temporaire. `bwrap --unshare-net` interdit le réseau externe pendant les tests et monte un répertoire scratch distinct sur le chemin `.runtime` compilé dans chaque binaire. Le runner vérifie également le refus d’une livraison inconnue dans ce scratch. Aucun fichier du `.runtime` du checkout n’est consulté par le test de processus.

Le CLI distingue les modes de sortie `webhook_delivery` et `direct_local`. L’aide décrit explicitement que le mode direct saute la lecture de livraison et n’authentifie pas son appelant. La sortie webhook dit seulement que l’empreinte fournie en argument correspond aux octets courants relus; elle n’affirme pas que l’appelant qui a fourni cette empreinte est un hôte authentifié.

## Couverture et limites

Ce résultat est un test processus local avec fixture, pas une intégration Forgejo live et pas le parcours produit complet. Il démontre une admission/claim idempotente; il ne modifie pas le code de la fixture, ne crée aucune pull request et ne passe par aucun vérificateur indépendant ou merge. Aucun runtime AgentOS/LLM n’a été appelé. La sonde locale à `http://127.0.0.1:11434/api/tags` a échoué le 2026-10-23 01:16 UTC; aucune autre intégration de modèle autorisée n’a été confirmée pendant ce lot.

La frontière de confiance reste insuffisante pour présenter l’inbox ou ce CLI comme autorité contre un agent de même UID. Le receiver vérifie le HMAC à l’ingestion, mais la valeur HMAC n’est pas stockée ni revérifiée à la lecture; le même UID peut réécrire l’inbox et reconstruire sa chaîne SHA-256. Les vérifications `0700/0600` limitent les remplacements ordinaires par d’autres UID, pas par un processus de même UID. Les options du CLI et le digest ne sont pas authentifiés; un agent qui peut lancer ce binaire peut fournir son propre binding. Cette exécution ne prouve donc pas une provenance ou une autorisation de l’hôte.

Le fichier `tests/forgejo_mission_delivery_process.rs` est ignoré par défaut pour le seul test qui relit `.runtime`; le runner le lance avec un montage scratch. Le test positif est aussi ignoré par défaut et exige le runner réseau isolé. Les autres tests de refus sans effet de bord restent dans `--all-targets`.

## Vérifications reproductibles

Versions observées : Rust/Cargo 1.95.0, Python 3.12.3, bubblewrap 0.9.0. Dépendances Rust épinglées dans `Cargo.toml` et `Cargo.lock`.

| Commande | Résultat |
| --- | --- |
| `python3 scripts/run_forgejo_mission_delivery_process_e2e.py` | 2 tests de processus passent : admission après refus fermé, puis refus d’une livraison inconnue; compilations offline/locked; deux espaces `.runtime` scratch sous bwrap. |
| `cargo test --offline --locked --all-targets` | 29 suites, 268 réussis, 0 échec, 9 ignorés. Les tests processus qui exigent bwrap sont exécutés séparément par leurs runners. |
| `cargo clippy --offline --locked --all-targets -- -D warnings -A clippy::too_many_arguments -A clippy::bool_assert_comparison -A clippy::manual_div_ceil` | Passe. Les trois catégories neutralisées sont des avertissements historiques déjà relevés dans les lots précédents. Le premier passage avait signalé un `collapsible_if` nouveau dans le test; il a été corrigé et le passage final est vert. |
| `cargo build --offline --locked --release --all-targets` | Passe. |
| `cargo fmt --all -- --check` | Passe après correction du format du test d’aide. |
| `git diff --check` | Passe pour les chemins suivis modifiés; les nouveaux Rust sont aussi couverts par `cargo fmt`, et le runner Python par l’analyse AST sans écriture. |
| `python3 -c 'import ast,pathlib; ast.parse(pathlib.Path("scripts/run_forgejo_mission_delivery_process_e2e.py").read_text())'` | Passe. |

Les sorties finales sont dans `evidence/webhook-cli-admission-001-{isolated-final,all-targets-final,clippy-final,release-build-final,rustfmt-final,diff-check-final,python-parse-final,untracked-whitespace-final}.log`; leurs empreintes figurent dans `evidence/webhook-cli-admission-001-sha256.txt`. `evidence/webhook-cli-admission-001-clippy-allowances.log` conserve l’échec initial du nouveau lint; le runner a aussi eu un premier échec de comptage de requêtes fixture, corrigé en accord avec les relectures répétées réelles.

Une revue indépendante read-only n’a relevé aucun autre défaut P1/P2 après avoir confronté le runner au journal d’exécution. Son doute initial sur l’ordre des mounts bwrap a été retiré : le journal montre les deux tests passés avec les chemins scratch sous `/tmp`, et le scénario a effectivement créé puis relu ses fichiers sous le montage.

## Suite proposée

Relier le claim à l’interface existante de `mission-012-mcp-host` et à son bac à sable de vérification, sans créer un second orchestrateur. D’abord définir qui, dans l’hôte, possède le binding, le journal et les capacités; ensuite tester une vraie petite modification sous la fixture et un échec actionnable suivi d’une correction. Une connexion de modèle ne sera exercée que si elle est déjà autorisée et joignable. La fusion contrôlée et la preuve Forgejo restent des jalons distincts.
