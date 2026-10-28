# WEBHOOK-APPEND-BEFORE-ACK-CRASH-RECOVERY-001 — rejeu après SIGKILL

Date : 26 octobre 2026. Travail local dans le worktree fortement modifié basé sur `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`. Aucun reset, commit, service externe, Forgejo live, modèle, secret réel ou changement de production.

## Lacune et test rouge

CONTINUATION-062 prouvait une reprise après arrêt gracieux; un autre parcours reprenait après SIGKILL une fois le receiver arrêté. Aucun test ne tuait le processus après `fsync` de l’événement et avant l’écriture de la réponse HTTP. Le nouveau test processus `abrupt_receiver_crash_after_durable_append_before_ack_replays_idempotently` cible cette fenêtre.

Avant l’ajout du failpoint, le test échoue comme attendu : le receiver renvoie `HTTP/1.1 202 Accepted` avec `duplicate=false`. Ce résultat rouge montre que le harnais ne créait pas encore la panne demandée; ce n’est pas un défaut du chemin de succès normal.

## Changement et comportement observé

Le receiver expose maintenant un failpoint uniquement sous la feature Cargo préexistante `test-crash-injection`. `crash_after_selected_webhook_append` ne tue le processus qu’en build debug, lorsque la livraison choisie vient d’être nouvellement ajoutée (`202`, `accepted=true`, `duplicate=false`) et avant `write_response`. L’append du journal se termine d’abord dans `EventJournal::append_locked`, qui écrit la ligne puis appelle `File::sync_all`.

Le test lance le vrai binaire `forgejo-webhook-local`, envoie une requête `issues/opened` HMAC signée avec un secret de fixture, et observe : aucune réponse HTTP, terminaison du receiver par `SIGKILL`, une seule ligne durable portant l’identifiant attendu, puis redémarrage avec le même journal. Le rejeu exact reçoit `202`, `duplicate=true`, séquence 1; le SHA-256 du journal est inchangé et il n’existe toujours qu’un événement. Enfin, `reconcile_inbox_events` consomme l’inbox relue après le rejeu et retrouve la livraison ainsi que l’état courant de la fixture (`open`).

Le contrôle d’exposition passe : `cargo build --release --locked --offline --bin forgejo-webhook-local` réussit sans la feature; `cargo check --release --locked --offline --features test-crash-injection --bin forgejo-webhook-local` échoue précisément sur le `compile_error!` ajouté au binaire receiver. Le mécanisme de crash ne peut donc pas être activé dans ce binaire release.

## Vérifications et portée

- Test ciblé avec `test-crash-injection` : 1/1 réussi après le changement; le test rouge précédent conservé.
- Cible complète `webhook_receiver_process_e2e` avec la feature : 29 réussis, 0 échec, 64 ignorés. Les 64 ignorés nécessitent le runner d’isolation réseau et ne sont pas comptés comme testés ici.
- Build release sans feature réussi; build release avec feature refusé par la garde attendue.
- `cargo fmt --all -- --check`, `git diff --check` et `python3 scripts/verify_cargo_license_state.py` passent. Le readback garde `license=null`, `license_file=null`, `publish=[]`, `no_license_selected`, droits `not_assessed`, marqueur `NOASSERTION` personnalisé.
- Cargo/rustc 1.95.0, Linux aarch64; commandes, sorties et empreintes sont archivées dans ce dossier.

Ce run simule la mort du processus sur un hôte vivant après `sync_all`; il ne simule pas perte d’alimentation, crash noyau, perte du cache disque, corruption ou troncature. Il n’exerce pas le retry natif de Forgejo : le test client renvoie explicitement la requête signée à l’instance receiver redémarrée. Un seul scénario déterministe (`n=1`) est observé; ce n’est ni une mesure de performance ni une preuve de durabilité sur tous les systèmes de fichiers.

Cargo reste sans licence produit choisie; les droits GitSpace demeurent indéterminés. Aucun runtime AgentOS/modèle local autorisé n’a été constaté; ce test n’utilise que le harness et le secret de fixture. Le benchmark apparié Forgejo + agent + CI reste non démontré et bloqué sans runtime comparable et plafond de coût fournisseur fiable. Projet incomplet.
