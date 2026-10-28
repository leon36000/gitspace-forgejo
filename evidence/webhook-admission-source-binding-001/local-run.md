# WEBHOOK-ADMISSION-SOURCE-BINDING-001 — reçu de source du handoff hôte

Date locale : 24 octobre 2026. Dépôt de travail : branche `local/WEBHOOK-005-canonical-links`, HEAD `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`. Le checkout est fortement modifié; aucune modification préexistante n’a été réinitialisée.

## Changement

Le handoff stable `MissionHostAdmissionV1` conserve son identité de mission à travers les redélivraisons. Une source distincte `MissionHostAdmissionWebhookSourceV1` relie chaque émission du handoff à l’UUID de livraison, à la séquence/identité/type/hash de l’événement d’inbox et au scope, issue, mission, allocation et hash d’exigences. `forgejo-mission-local` écrit ce sidecar dans le `.runtime` privé à côté du DTO, avec un nom dérivé de l’UUID. Le schéma host config passe de 4 à 5; `admission_source_mode` choisit explicitement `direct_local` ou `webhook_delivery`.

En mode webhook, le host exige le fichier source et l’inbox privée hors des espaces de travail. Il relit l’inbox avec `EventJournal::replay_read_only`, puis compare l’événement et ses bindings avant la création du reçu d’exécution et avant toute trame MCP. La config directe continue d’exiger explicitement `webhook_inbox_file=null` et `admission_source_file=null`.

## Tests et résultats

- Le journal rouge `red-process.log` a exécuté l’E2E processus avant l’émission du sidecar : l’assertion échouait parce que `host_admission_source_file` n’existait pas.
- `library-contract.log` : le contrat confirme que deux livraisons du même issue binding conservent le même DTO d’admission et donnent des reçus sources différents; le bon événement est relu et une empreinte altérée est refusée (1/1).
- `config-v5-contract.log` : le producteur de config directe émet le schéma v5 et n’intègre pas l’admission au fichier config (1/1).
- `host-process-direct.log` : le véritable binaire host refuse un sidecar manquant et un hash périmé avec code 65, sans créer le reçu d’exécution. Avec le sidecar valide, le processus atteint `initialize`; la fin sans tests d’acceptation est correctement marquée `baseline_not_verified` (1/1).
- `host-process-isolated.log` rejoue ce test unique sous le runner Bubblewrap du dépôt, avec `--unshare-net`, HOME masqué, registre Cargo hors ligne et binaires issus du code courant (1/1, 24 tests du binaire filtrés). Ce test vérifie le host et ses refus, pas l’exécution de `cargo.test`.
- `all-targets.log` : `cargo test --offline --locked --all-targets`, 436 réussis, 0 échec, 28 ignorés, 34 groupes. Les tests ignorés ne sont pas comptés comme exécutés.
- `release-build.log`, `rustfmt.log`, `diff-check.log`, `untracked-whitespace-check.log`, `python-compile.log`, `cargo-license-state.json` et `cargo-license-tests.log` : build release, formatage, contrôles d’espaces, compilation Python et garde licence passent. Cargo garde `license=null`, `license_file=null`, `publish=[]`, `license_decision=unselected` et `redistribution_rights=not_assessed`.
- Le Clippy strict initial est conservé dans `clippy.log`; il échoue sur des avertissements existants (`too_many_arguments`, `collapsible_if`, `bool_assert_comparison`). Le premier passage avec quatre allowances, `clippy-with-baseline-allowances.log`, a aussi relevé `enum_variant_names` et `type_complexity` dans des tests existants. `clippy-with-baseline-allowances-final.log` passe `cargo clippy --offline --locked --all-targets -- -D warnings` avec les six catégories explicitement neutralisées : `too_many_arguments`, `bool_assert_comparison`, `manual_div_ceil`, `collapsible_if`, `enum_variant_names` et `type_complexity`.

Commandes principales : `cargo test --offline --locked --test webhook_mission_admission_contract webhook_source_receipt_revalidates_exact_event_without_changing_stable_handoff`; `cargo test --offline --locked --test mission_012_mcp_lifecycle admission_handoff_is_versioned_and_host_config_has_no_copied_claim_fields`; le processus host ciblé est relancé avec `--exact --ignored --nocapture` sous le runner du dépôt; suite générale `cargo test --offline --locked --all-targets`; `cargo build --offline --locked --release`; `cargo fmt --all -- --check`; Clippy comme ci-dessus; `python3 -m py_compile scripts/run_mission_012_live_mcp_agent_e2e.py`. `host-process-isolated.log` et `full-mission-process-blocked.log` contiennent les invocations Bubblewrap produites par le runner et leurs sorties. Les sorties vides de rustfmt/diff-check signifient que ces commandes n’ont rien écrit; leurs codes de sortie sont 0.

## Blocage E2E restant

`full-mission-process-blocked.log` est l’essai du parcours MISSION-012 plus large sous le runner hors réseau. Le CLI d’admission produit le sidecar, le host valide le reçu, transmet le contexte sourcé, répond aux appels MCP et refuse les capacités non autorisées. Le premier `cargo.test` reste inconclusif parce que le `systemd-run` du host ne trouve pas le bus utilisateur dans le namespace Bubblewrap (`Failed to connect to bus: No medium found`); l’E2E finit donc en échec avec exit 2 au lieu de démontrer le rouge/vert de l’agent. Ce n’est pas une preuve que le candidat a été testé.

Décision de sécurité : le bus n’a pas été monté dans la namespace du runner. Le montage donnerait une capacité D-Bus au binaire de test et aux programmes compilés depuis les sources examinées. Prochaine action : fournir au host un superviseur extérieur derrière une capacité locale étroite, avec commande fixe, arguments/chemins bornés, quota de lancement et sortie bornée; le processus candidat reste dans la namespace sans bus. Cette interface est seulement conçue comme prochaine étape, pas implémentée ni testée. Le lot n’a pas affaibli l’isolation réseau.

## Frontière de confiance et portée

Ce sidecar est un reçu de traçabilité, pas une signature ni une preuve de correction. L’ingestion vérifie le HMAC, mais le HMAC original n’est pas persisté et n’est pas recalculé en relecture. Le hash-chain du journal détecte les incohérences accidentelles; un acteur local du même UID capable de réécrire le journal et le sidecar reste dans la frontière de confiance. Aucun test de Forgejo réel, LLM, AgentOS, RAG/mémoire, fusion ni comparaison Forgejo+agent+CI n’est ajouté ici.

Aucun code GitSpace ou Forgejo n’est copié. Aucun secret ou payload privé n’est envoyé à un service externe. Aucun accès, déploiement, publication ou changement de production n’a été effectué.

Toolchain, sorties, manifeste local `SHA256SUMS` et empreintes des sources modifiées : voir les fichiers voisins de ce rapport.

## Empreintes

Le manifeste local couvre 21 éléments de preuve (hors le manifeste lui-même) et passe 21/21. Le manifeste global compte 2 683 lignes : 2 682 entrées SHA-256 valides, toutes présentes et concordantes; le seul élément non SHA-256 est le placeholder historique `None  tests/test_mission_012_live_composed_e2e.py`, préservé.
