# CONTINUATION-128 — refus du replay CLI sans réparer l’inbox

Date : 26 octobre 2026. Worktree `/home/pc6/Documents/Codex/2026-10-21-mission-construire-une-forge-con-ue/.runtime/webhook-005-workspace`, branche de travail `local/WEBHOOK-005-canonical-links`, HEAD de base `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`. L’arbre était déjà fortement modifié; aucun commit, reset ou nettoyage n’a été fait. Les hashes et la toolchain de ce lot sont dans les fichiers voisins.

## Changement

`EventJournal::replay_read_only()` est maintenant accessible au binaire host. Il attend sous le même verrou borné avant d’inspecter le fichier, exige le propriétaire/permissions/nombre de liens attendus, applique la limite d’inbox, et valide sans ajouter de newline ni tronquer une queue finale non terminée. Le chemin absent est retourné comme inbox vide seulement après le verrou, ce qui conserve le comportement d’attente annoncé quand un autre processus tient le verrou.

`forgejo-reconcile-local` utilise cette lecture. Toute chaîne ou ligne corrompue devient une réponse single/batch `inconclusive/inbox_journal_invalid`, code de sortie 1, `journal_repair_attempted=false`; l’action demande de préserver une copie et de restaurer une chaîne complète validée avant nouvelle tentative. L’erreur est rendue avant la lecture du FD credential et avant la construction du client Forgejo. `EventJournal::replay()` conserve sa récupération historique pour le chemin receiver/redelivery; aucun basculement de cette responsabilité n’est effectué dans le CLI d’autorité.

## TDD et vérifications

Le test `real_cli_preserves_an_incomplete_tail_before_quarantine_validation` a été ajouté avant le changement de code. Avec l’ancien `replay()`, il a échoué sur la comparaison byte-for-byte : le suffixe synthétique `{"sequence":4,"payload":` avait été retiré avant que le CLI réponde `invalid_quarantine_disposition`. Le premier résultat rouge n’avait pas été redirigé vers un fichier brut; l’observation est transcrite dans `tdd-red-summary.txt`. Le test final construit une chaîne complète de trois événements (source invalide, disposition malformée, livraison active), ajoute une queue sans newline et lance le vrai exécutable avec un FD pipe contenant une valeur synthétique. Il vérifie le refus JSON, l’absence de hash/champ malformé dans la sortie, le credential non consommé et l’inbox identique octet pour octet.

Le run ciblé `webhook_quarantine_cli` passe 3/3 après changement. Le contrat de config passe 4/4 après correction de sa fixture de créer l’inbox vide en mode privé 0600, conforme au validateur read-only. Le test de verrou passe 3/3 après déplacement de la vérification des métadonnées sous le verrou; les premiers all-targets avaient exposé ces deux compatibilités et leurs logs sont conservés.

Vérification finale sur le code courant : `cargo test --offline --locked --all-targets` passe 622, échoue 0, ignore 106, sur 36 groupes. Les tests ignorés ne sont pas exécutés par cette commande. `cargo fmt --all -- --check`, `git diff --check`, `python3 scripts/verify_cargo_license_state.py` et readback `cargo metadata --offline --locked --no-deps --format-version 1` réussissent. Les logs antérieurs en échec sont conservés, suivis du log final réussi.

Le run final de référence est `all-targets-verified.log`; le processus `cargo test` s’est terminé avec exit code 0. Il contient tous les résultats par target et confirme 622 réussis, aucun échec, 106 ignorés sur 36 groupes.

## Limites

Le journal, sa queue finale, la disposition et le credential sont synthétiques. Il n’y a eu ni instance Forgejo, ni modèle/LLM, ni AgentOS, ni réseau externe, ni coût. L’ordre de code et la sortie CLI vérifient que le réconciliateur refuse avant son client amont; ce n’est pas une capture réseau. Le verrou est coopératif : un writer local qui le contourne n’est pas fenced. Le refus ne donne pas de réparation; il faut une procédure distincte qui préserve et valide une copie avant le prochain essai. Les permissions strictes renforcent la sécurité, mais peuvent refuser un inbox mal configuré au lieu de le lire.

Cargo demeure `license=null`, `license_file=null`, `publish=[]`, `no_license_selected`; droits de redistribution non évalués et export SPDX `NOASSERTION`. La licence GitSpace demeure indéterminée et aucun code tiers n’est copié ou adopté. Aucun gain comparatif n’est revendiqué; les critères cumulatifs du projet restent ouverts.

Prochaine action de travail : tester depuis le processus agent sandboxé les routes Forgejo API/Web et receive-pack et l’accès au credential de fusion, tout en confirmant que le gate conserve son trafic autorisé. Le test doit prouver le filtrage imposé par la frontière réseau du processus, et pas seulement l’allowlist du client Rust.
