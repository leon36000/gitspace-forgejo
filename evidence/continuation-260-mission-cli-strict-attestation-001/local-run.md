# CONTINUATION-260 — attestation avant credential dans la CLI mission

Date locale : 27 octobre 2026. Worktree `/home/pc6/Documents/Codex/2026-10-21-mission-construire-une-forge-con-ue/.runtime/webhook-005-workspace`, HEAD de base `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`, worktree fortement modifié avant ce lot. Aucun changement préexistant n’a été nettoyé, écrasé ou commité. Le lot ne touche ni l’instance de production ni le code Forgejo upstream.

## Lacune et changement

Le `run()` de `src/bin/forgejo-mission-local.rs` lisait le fichier de credential avant d’ouvrir le journal de livraison. La lecture de l’événement passait ensuite par `read_journaled_webhook_event`, qui vérifie la chaîne mais pas l’attestation Ed25519. Cela laissait l’admission webhook consommer un credential avant d’authentifier la ligne reçue.

En mode `--delivery-id`, le CLI exige maintenant 1 à 8 clés publiques canoniques (`--trusted-webhook-attestation-public-key-hex`, répétable). Les clés sont décodées et contrôlées avant tout accès au credential. La CLI ouvre le journal, valide la signature et lie l’événement à l’issue du profil avant la lecture du credential; elle relit puis revalide cette source avant toute requête Forgejo. Les refus distinguent politique manquante/invalide, attestation manquante, clé non approuvée, signature invalide et version inconnue. Les fixtures mission et receiver-process utilisent maintenant des lignes signées. Le runner Bubblewrap inscrit les nouveaux tests isolés.

La première preuve rouge `mission-cli-policy-red.log` lance le binaire avant correction avec une clé absente et un chemin de credential inexistant. Il retourne `mission_host_credential_invalid` au lieu de `host_journal_attestation_policy_missing`, preuve que le credential était traité avant la politique. La deuxième rouge `mission-cli-untrusted-key-red.log` construit une ligne HMAC/Ed25519 réellement signée et fournit une autre clé; avant d’ajouter l’option au parser, le test reçoit `mission_arguments_invalid` sur le flag inconnu. Elle identifie la nouvelle interface manquante, pas encore un refus cryptographique. La première tentative de lot vert `mission-cli-policy-process-green.log` échoue sur l’ancien test d’ID inconnu, qui ne fournissait pas la nouvelle clé requise. Le fixture a été migré; cet échec est conservé, puis les trois cas passent dans `mission-cli-policy-process-rerun-001.log`.

## Vérifications exécutées

- `cargo test --offline --locked --bin forgejo-mission-local` : 36 réussis, 0 échec, 0 ignoré.
- `python3 scripts/run_forgejo_mission_delivery_process_e2e.py --only launched_cli_requires_attestation_policy_before_reading_host_credential --only launched_cli_rejects_untrusted_attestation_before_reading_host_credential --only launched_cli_refuses_an_unknown_delivery_without_creating_runtime_state` : 3 tests processus isolés réussis individuellement sous Bubblewrap `0.9.0`, `--unshare-net --unshare-pid`. Cela couvre politique absente, signature réelle vérifiée contre la mauvaise clé avant le chemin de credential inexistant, et livraison absente avec politique valide.
- `python3 scripts/run_forgejo_mission_delivery_process_e2e.py --only actual_mission_cli_admits_signed_webhook_after_closed_refusal_and_reuses_allocation --only actual_mission_cli_refuses_same_revision_requirement_edit_after_signed_delivery` : 2 tests receiver→inbox→CLI réussis sous Bubblewrap. Le premier vérifie l’issue fermée, une admission valide, reprise idempotente et conflit de contexte; le second refuse un digest d’exigences devenu périmé.
- `cargo test --offline --locked --test forgejo_mission_delivery_process` : 2 tests ordinaires réussis, 3 ignorés parce qu’ils exigent le runner isolé.
- `cargo test --offline --locked --test forgejo_mission_delivery_process_e2e --no-run` : compilation du test processus ciblé réussie.
- `cargo check --offline --locked --all-targets`, `cargo fmt --all -- --check`, `git diff --check` et `python3 -m py_compile scripts/run_forgejo_mission_delivery_process_e2e.py` réussissent.

Les logs correspondants sont archivés dans ce répertoire. Le code base HEAD n’a pas changé; les tests exécutés sont ciblés et ne représentent pas la suite complète du produit. Les journaux de sous-processus isolés donnent leur propre `source_tree_sha256`; `source-sha256.txt` lie le rapport aux sources finales de ce lot.

## Versions et portée observée

- Linux `7.0.0-1019-nvidia-aarch64-with-glibc2.39`, arm64.
- `rustc 1.95.0 (59807616e 2026-04-14)`, `cargo 1.95.0 (f2d3ce0bd 2026-03-21)`.
- Bubblewrap `0.9.0`; Python de runner 3.12 (voir `environment.json`). Dépendances Cargo verrouillées et hors ligne.
- Les deux tests receiver→CLI utilisent un serveur HTTP loopback de fixture pour simuler l’API Forgejo; aucune instance Forgejo ni CI Actions n’a été lancée dans CONT-260.

## Limites de confiance et suite

La clé publique est transmise à cette CLI de banc par les arguments du lanceur. Elle n’est pas secrète, mais son origine doit être protégée par le lanceur hôte; si un appelant non fiable contrôle les arguments du binaire, il peut définir sa propre clé. Il faut garder la CLI hors de portée de l’agent et l’intégrer à un superviseur qui fournit l’allowlist depuis une source protégée. La répétition de la vérification ferme le chemin étudié avant les appels réseau, mais n’ajoute pas de verrou transactionnel contre un autre processus du même UID qui écrit dans l’inbox entre les opérations. `forgejo-reconcile-local` vérifie toujours la chaîne legacy sans attestation. Rotation, révocation, migration des anciennes lignes et anti-rollback restent ouverts.

Ce lot n’exécute pas de modèle, AgentOS, PR, gate d’acceptation, merge, Forgejo live ou comparaison conventionnelle. Aucun résultat de gain n’est inféré. Prochaine action locale : appliquer la même exigence de clé hors journal au reconciler et prouver son rejet d’une attestation absente/non approuvée avant credential et requêtes, tout en gardant explicite la frontière du lanceur de confiance. Le projet reste incomplet.
