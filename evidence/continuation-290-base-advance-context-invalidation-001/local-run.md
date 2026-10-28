# CONTINUATION-290 — refus de contexte périmé et reprise d’admission

Date d’exécution : 27 octobre 2026. Worktree de départ : `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`; dépôt local très modifié, sans commit créé.

## Question vérifiée

Si la branche protégée avance entre la lecture du contexte d’une mission et son admission, le CLI refuse-t-il ce contexte périmé sans créer de revendication ou de transfert? Le même ID de livraison/allocation peut-il ensuite reprendre en lisant le contexte correspondant à la nouvelle base?

## Parcours exécuté

Le test processus `actual_mission_cli_rejects_stale_context_then_resumes_same_allocation_after_base_advance` lance le vrai receiver `forgejo-webhook-local` et le vrai CLI `forgejo-mission-local`. Une fixture HTTP loopback fournit l’API synthétique. Le receiver accepte une livraison `issues/opened` signée avec le secret synthétique du test. Le CLI lit l’issue, la branche et les exigences, puis récupère une source au commit `0123456789abcdef0123456789abcdef01234567`. La fixture avance la branche vers `fedcba9876543210fedcba9876543210fedcba98` après cette lecture.

La première admission rend le verdict `inconclusive` avec le code `context_stale`, contenant l’ancien `context_commit` et le nouveau `workspace_base`. Elle persiste l’unique événement `mission_generation_allocated_v1` pour `admission-41`, mais ne crée ni `mission_execution_claim_v1` ni fichier handoff. L’inbox contient toujours une seule livraison.

Le même ID de livraison et d’allocation sont ensuite réessayés. Le CLI relit les exigences et le contexte à la nouvelle base, rejoue la réservation stable comme génération 1 (`allocation_replayed=true`), crée la revendication et écrit un `MissionHostAdmissionV1` validé lié à la nouvelle base. Le journal contient alors exactement l’allocation et la revendication; l’inbox reste inchangée. Le test asserte aussi les chemins des requêtes de source ancienne puis les lectures d’arbre et d’exigences à la base avancée.

Le runner met le code source en lecture seule dans Bubblewrap, monte un `.runtime` privé, désactive le réseau et isole les PID (`--unshare-net --unshare-pid`). Le binaire de test est ignoré par Cargo et lancé explicitement par le runner. Résultat actuel : **1 test, 1 réussi, 0 échec**, journalisé dans [`recovery-renamed-final.log`](recovery-renamed-final.log).

## Vérifications de l’arbre courant

`cargo test --locked --offline --all-targets` : **694 réussis, 0 échec, 153 ignorés en 37 blocs**. Les tests marqués ignorés, dont les tests processus, ne sont pas exécutés par cette commande; le scénario ci-dessus est exécuté séparément par le runner isolé.

Les contrôles `cargo fmt --all -- --check`, `rustfmt --edition 2024 --check src/mission_admission.rs tests/forgejo_mission_delivery_process_e2e.rs`, compilation syntaxique du script Python, `python3 scripts/verify_cargo_license_state.py` et `git diff --check` réussissent. Le guard conserve `license=null`, `license_file=null`, `publish=[]`, `no_license_selected`, `redistribution_rights=not_assessed` et `NOASSERTION`.

## Ajustement d’hypothèse et limites

La première hypothèse voulait reporter la réservation durable de génération jusqu’après la validation de tout le contexte. L’essai initial a constaté que le code réservait déjà la génération avant le refus de contexte; un changement temporaire d’ordre a fait passer une version intermédiaire du scénario, mais l’exécution complète a échoué sur le contrat préexistant `failed_context_validation_can_resume_with_the_same_reserved_generation`. Ce contrat préserve la même génération lors d’une reprise. Le changement runtime temporaire a donc été retiré; le test final exige une allocation stable sans claim/handoff lors du refus, puis une reprise idempotente de cette même allocation. Les logs `red.log`, `green.log`, `green-final.log`, `cargo-all-targets.log` et `recovery-final.log` sont conservés comme historique et ne représentent pas tous le comportement final. Le dernier run renommé et la suite complète sont les reçus finaux.

Ce test emploie les vrais binaires receiver et admission, mais ses réponses HTTP viennent d’une fixture loopback, pas d’un serveur Forgejo. Il couvre un seul scénario sur Linux/aarch64, avec contenu et secret synthétiques. Il ne démontre ni atomité de la branche sur Forgejo, ni concurrence d’un writer réel, ni sécurité d’un déploiement; aucun LLM, AgentOS, CI externe, coût modèle ou service externe n’est utilisé. Aucun code runtime n’a été changé dans le lot final.

Ce lot améliore une preuve locale de refus et de reprise. Il ne valide pas tous les critères du projet, ne démontre aucun gain comparatif et ne marque pas la mission comme terminée.

## Reçus

- [`commands.txt`](commands.txt) — commandes exécutées.
- [`tool-versions.txt`](tool-versions.txt) — versions et environnement.
- [`source-sha256.txt`](source-sha256.txt) — empreintes des sources/docs consultés pour ce lot.
- Les reçus finals `cargo-all-targets-renamed-final.log`, `recovery-renamed-final.log`, `cargo-fmt-renamed-final.log`, `rustfmt-renamed-final.log`, `python-compile-renamed-final.log`, `cargo-license-guard-renamed-final.json` et `git-diff-check-renamed-final.log`.
