# WEBHOOK-HOST-AUTHORITY-SYNTAX-001 — syntaxe Host refusée avant ingestion

Date : 25 octobre 2026. Worktree produit `local/WEBHOOK-005-canonical-links`, basé sur `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`. Le run n’a pas démarré Forgejo et n’a utilisé que le vrai processus local `forgejo-webhook-local`, un listener loopback éphémère, des payloads HMAC synthétiques et une inbox temporaire.

## Défaut reproduit et correction

Le test processus a d’abord échoué contre le parseur existant : cinq valeurs Host mal formées (`forgejo.local/path`, port alphabétique, littéral entre crochets invalide, espace interne et userinfo) ont chacune reçu HTTP 202 et modifié l’inbox. Le test rouge est conservé dans `red.log`; code de sortie 101.

Le parseur rejette maintenant ces autorités avant l’ingestion. Il valide un sous-ensemble ASCII URI : reg-name avec échappements `%HH`, IPv6/IPvFuture entre crochets et port composé de chiffres. Il rejette notamment caractères de chemin, userinfo, espaces internes, échappements percent incomplets et littéraux IPv6 invalides. Ce n’est pas un canonicaliseur URI général : pas de nom Unicode brut/IDNA, pas d’identifiant de zone IPv6 et aucune validation de plage de port.

## Vérification

- `webhook_http_contract`: 6 réussis, 0 échec, dont matrice d’autorités valides et mal formées.
- `webhook_receiver_process_e2e`: 12 réussis, 0 échec, 40 ignorés par garde explicite d’exécution sous Bubblewrap; le nouveau test processus passe et vérifie HTTP 400 `request_malformed` ainsi qu’une inbox byte-à-byte inchangée.
- `cargo fmt --all -- --check` et `git diff --check`: code retour 0.
- Readback Cargo: `license=null`, `license_file=null`, `publish=[]`; décision `no_license_selected`, droits `not_assessed`. Le garde Cargo passe ses 7 tests.
- Toolchain : Rust/Cargo 1.95.0.

Les commandes et sorties sont dans ce dossier. RFC de référence : [RFC 9112 §3.2](https://www.rfc-editor.org/rfc/rfc9112.html#section-3.2). La vérification est locale et ciblée; elle n’établit pas la conformité complète d’un serveur HTTP, un test Forgejo live, une exécution LLM ou un gain comparatif.

## Suite

Poursuivre la revue des writers LFS Forgejo dans le scratch avant tout choix de fork. Au moment de ce run, aucune commande `go` n’est disponible dans PATH ni dans `/usr/local/go/bin`, `/opt/go/bin`, `/home/pc6/go/bin` ou `/home/pc6/.local/bin`; aucun outil n’a été installé. Les tests Go POC restent donc non exécutables dans cet environnement tant qu’un toolchain existant n’est pas retrouvé.
