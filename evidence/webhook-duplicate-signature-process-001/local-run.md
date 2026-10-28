# WEBHOOK-DUPLICATE-SIGNATURE-PROCESS-001 — refus composé d’une signature dupliquée

Date : 25 octobre 2026. Worktree `local/WEBHOOK-005-canonical-links`, HEAD `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`. Le test ajouté à [webhook_receiver_process_e2e.rs:431](../../tests/webhook_receiver_process_e2e.rs#L431) passe par le vrai binaire `forgejo-webhook-local`, un listener loopback éphémère et une inbox temporaire privée.

La requête contient un corps d’issue HMAC-valide et deux champs `X-Forgejo-Signature` identiques. Le processus répond `401 signature_missing` avec `retryable=false`; l’inbox reste octet par octet identique à son état antérieur, puis son replay est vide. Un contrat parser vérifiait déjà le doublon isolément; ce scénario ajoute la preuve au niveau du processus jusqu’au journal durable. Aucun défaut de comportement n’a été trouvé dans l’implémentation pendant ce lot : c’est un contrôle composé ajouté, pas une correction runtime ni une exécution rouge/verte.

## Vérifications

- Le test processus ciblé passe 1/1 (journal `focused-test.log`, code 0).
- `cargo test --offline --locked --all-targets -- --test-threads=1` : le journal complet donne 34 groupes `ok`, 500 réussis, 0 échec, 55 ignorés. Après cette exécution, seule la mise en forme d’une assertion a changé; le test ciblé et `cargo fmt --all -- --check` ont ensuite repassé sur la source finale.
- `cargo fmt --all -- --check`, `git diff --check` et `python3 scripts/verify_cargo_license_state.py` passent. Le readback garde `license=null`, `license_file=null`, `publish=[]`, `no_license_selected`, droits `not_assessed`.
- Source du test : SHA-256 `c45df26cfc2c77ade91cb727d63f75f02982ae18bb957a8b4475176ab2f43753`.

Les traces, commandes, outils, HEAD et empreintes sont dans ce dossier. Le test utilise seulement le vrai récepteur local et des données synthétiques; Forgejo vivant, modèle/AgentOS, comparaison de valeur et vérification de l’acceptation par le projet hôte ne sont pas démontrés ici. Les 55 cas ignorés n’ont pas été rejoués dans la commande all-targets; leurs raisons et preuves antérieures restent dans leurs paquets respectifs.

## Suite

Continuer le laboratoire séparé Forgejo v17 sur une précondition obligatoire partagée entre les writers, puis le cycle LFS durable de staging, promotion, reprise et nettoyage. Aucun patch scratch n’est adopté. Le parcours LLM et la comparaison contrôlée Forgejo + agent + CI conventionnels restent non démontrés; l’objectif global reste incomplet.
