# INBOX-CAPACITY-001 — limite cumulative de l’inbox webhook

## Portée et provenance

Lot développé dans le worktree local isolé `.runtime/webhook-005-workspace` le 23 octobre 2026. Au début du lot, `HEAD` était `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`; l’arbre était déjà modifié par des travaux antérieurs. Aucun changement antérieur n’a été réinitialisé, supprimé, publié ou commité. Les contrôles portent sur l’état de travail résultant; le hash HEAD ne désigne donc pas le contenu exact des fichiers testés.

Environnement : Linux aarch64, `rustc 1.95.0 (59807616e 2026-04-14)`, Cargo 1.95.0 (`f2d3ce0bd`, 2026-03-21). Détails dans `webhook-inbox-capacity-001-environment.log`.

## Changement

L’audit du journal a confirmé qu’avant ce lot les tailles de requête et de lot de réconciliation ne limitaient pas le journal cumulatif, lequel était relu entièrement avec `fs::read`. Le lot ajoute un plafond provisoire fixe de 64 MiB pour l’inbox webhook :

- l’ouverture dédiée `EventJournal::open_webhook_inbox` borne lecture et append; les écritures webhook restent plafonnées même si le handle initial a été ouvert par l’API générique;
- la taille du record sérialisé, y compris le saut de ligne, est contrôlée sous verrou avant l’écriture, puis la taille du fichier ouvert est contrôlée avant `write_all`;
- une livraison déjà présente et identique reste idempotente au plafond exact; une nouvelle livraison trop grande est refusée sans accroître le journal;
- le replay contrôle la taille avant allocation/lecture, utilise `O_NOFOLLOW`, contrôle fichier régulier/propriétaire/lien unique et lit au plus `limite + 1`; un historique excédentaire est refusé, jamais tronqué;
- un replay valide n’exige que l’accès en lecture. Pour réparer une dernière ligne incomplète ou ajouter son saut de ligne, le code rouvre en écriture avec `O_NOFOLLOW`, contrôle le propriétaire/type/liens et compare le périphérique/inode avec le descripteur lu; la taille doit toujours correspondre au contenu scanné. Le test remplace le chemin par un lien symbolique et vérifie que les deux opérations refusent la mutation et laissent la cible protégée intacte;
- le receiver refuse de démarrer avant le bind si l’inbox dépasse le quota; la réconciliation et l’admission transmettent une erreur actionnable; le handler renvoie HTTP 507 `inbox_capacity_exceeded`, non réessayable; `--capabilities` publie le quota et la politique.

Le comportement de compaction/rotation n’est pas implémenté : supprimer des lignes briserait potentiellement la chaîne de hashes, les séquences et la déduplication. Une inbox historique trop grande exige une sauvegarde, une migration/compaction vérifiée et une redélivrance opérateur. Le quota reste non configurable et provisoire. Les journaux génériques de mission qui n’utilisent pas le constructeur borné ne sont pas plafonnés.

## Vérifications locales

Commandes finales, toutes hors ligne et verrouillées :

- `cargo test --offline --locked --lib bounded_tail_repairs_refuse_a_replaced_symlink_path -- --nocapture` — 1/1;
- `cargo test --offline --locked --lib bounded_replay_reads_a_valid_journal_without_write_permission -- --nocapture` — 1/1;
- `cargo test --offline --locked --test webhook_inbox_capacity -- --nocapture` — 6/6;
- `cargo test --offline --locked --all-targets` — 310 réussis, aucun échec;
- `cargo clippy --offline --locked --all-targets -- -D warnings -A clippy::too_many_arguments -A clippy::bool_assert_comparison -A clippy::manual_div_ceil` — réussi avec ces trois catégories explicitement autorisées;
- `cargo build --offline --locked --release --all-targets` — réussi;
- `rustfmt --edition 2024 --config skip_children=true --check ...` sur tous les fichiers Rust concernés — réussi;
- `git diff --check` — réussi.
- `sha256sum -c evidence/webhook-inbox-capacity-001-sha256.txt` après la mise à jour du rapport et du checkpoint — toutes les empreintes listées correspondent.

Le test ciblé de l’inbox vérifie découverte du quota, dépassement par les chemins receiver et reconcile sans mutation, refus avant bind, réponse HTTP 507, doublon à quota exact et refus d’une réparation qui dépasserait le plafond. Les tests unitaires vérifient qu’un replay valide ne requiert pas le droit d’écriture et que les deux réparations refusent un chemin remplacé par un lien sans tronquer sa cible. Les logs de commande sont archivés dans ce répertoire; `webhook-inbox-capacity-001-sha256.txt` en donne les empreintes.

## Limites de preuve

Les refus oversized sont testés au moyen d’un fichier étendu au-delà du seuil et les chemins de processus locaux sont exercés. Le test HTTP utilise une limite de fixture nulle; il ne transmet pas 64 MiB de trafic. Aucun serveur Forgejo live, événement livré par Forgejo, backup/restore, compaction, redélivrance après 507, course hostile d’un autre processus sous le même UID, ni parcours avec LLM n’a été testé dans ce lot. Les journaux de fichiers et `flock` ne constituent pas une isolation de privilèges contre un processus de même UID.

La réparation sûre et ses tests ont été introduits en TDD : le test rouge ne compilait pas tant que les fonctions de réouverture protégée et de comparaison d’inode n’existaient pas; le test vert confirme le refus des deux mutations sur un chemin remplacé. Le test dédié de replay valide avec permissions sans écriture passe séparément. Voir `webhook-inbox-capacity-001-repair-race-red.log`, `webhook-inbox-capacity-001-repair-race-green.log` et `webhook-inbox-capacity-001-readonly-replay.log`.

## Revue indépendante

La revue indépendante n’a pas encore rendu de conclusion. Claude Code CLI 2.1.309 a confirmé Opus 5.5 à effort maximal, sous la session Claude Max déjà authentifiée; aucune clé Console/API n’a été configurée ni utilisée. Le travail se déroule dans `/tmp/forgejo-webhook-capacity-review.MjnADY`, copie filtrée des sources et tests concernés, sans historique Git, runtime, données Forgejo privées ni secrets. Le plugin officiel Ralph est actif, plafonné à trois itérations; le prompt demande une seconde passe par l’agent Claude configuré avec l’alias `sonnet`, limité à Read/Glob/Grep.

Le prompt de revue a conservé le nom de travail provisoire `WEBHOOK-007`, tandis que le lot a été renommé `INBOX-CAPACITY-001` pour éviter le conflit avec une tranche existante. La copie filtrée de `src/lib.rs` et du test a été rafraîchie après le dernier changement de code. Aux relevés du moniteur (06:54–06:56, heure locale), le processus était toujours présent, Ralph restait à l’itération 1/3, aucun résultat n’était visible et le hash de `src/lib.rs` avait changé depuis le lancement. Cette surveillance ne lit ni argv, ni environnement, ni prompt, ni terminal/session. Le résultat indépendant reste donc **en attente**; aucun constat Claude/Sonnet ni verdict d’absence de défaut n’est revendiqué.
