# WEBHOOK-PUSH-BRANCH-RECONCILE-001 — preuve locale

Date du contrôle : 25 octobre 2026. Worktree local `local/WEBHOOK-005-canonical-links`, HEAD `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`; arbre de travail fortement modifié. Le commit identifie seulement la base du checkout : les sources testées étaient des modifications locales non commitées. Ce sous-lot n’a pas changé le code produit; il a complété la vérification et la traçabilité d’un comportement déjà présent.

## Verdict de la tranche

**Pass pour le contrat local et le scénario processus ciblé; portée limitée à une fixture HTTP.** Une notification `push` périmée pour `refs/heads/feature/release/agent-proof` est réconciliée depuis l’état courant lu par le CLI. Le test processus observe le vrai récepteur HMAC, l’écriture durable de l’inbox, le vrai binaire de réconciliation et une fixture HTTP loopback. Un GET vers le chemin de branche encodé est fait; l’API de fixture fournit le SHA courant, différent de `payload.after`; aucun en-tête Authorization n’est envoyé; la réponse est `current_state_read`; le journal d’entrée reste octet pour octet inchangé.

Le contrat ciblé passe **29/29**. Le test processus `actual_reconcile_cli_process_reads_current_branch_after_stale_push_webhook` passe **1/1**, avec les 59 autres cas du binaire filtrés. Les tests de contrat vérifient aussi les refs non branche et les identifiants malformés avant appel API, l’ambiguïté d’un 404 (résultat inconclusif), ainsi que le nom exact de la branche et un SHA valide.

## Ce que la preuve ne démontre pas

- La fixture est un serveur HTTP loopback synthétique, pas Forgejo vivant; elle ne démontre pas l’authenticité d’une réponse Forgejo ni les permissions de l’API.
- Bubblewrap 0.9.0 utilise `--unshare-net --unshare-pid` pour ce scénario. Cela prouve l’isolation réseau du test exécuté dans ce bac à sable, pas celle d’un futur agent ni la séparation du credential entre exécuteur, vérificateur et gate.
- Aucun LLM, AgentOS, RAG, mémoire ou endpoint de modèle n’a été appelé. `AgentRuntimeV1` est un contrat; l’unique implémentation trouvée est `FixtureRuntime` dans les tests.
- Ce résultat ne démontre ni le parcours complet de livraison par LLM, ni une mesure comparative, ni une amélioration générale.
- La suite `--all-targets` n’a pas été rejouée pour ce checkpoint. Les fichiers historiques `all-targets.log` et `clippy-allowed.log` contiennent respectivement des échecs de tests de configuration et de lint en mode erreurs; ils sont conservés et ne sont pas utilisés pour conclure. Les fichiers `all-targets-final.log` et `clippy-final.log` sont également historiques, sans nouvelle exécution dans ce sous-lot.

## Commandes et sorties

Les commandes, toolchain et résultats synthétiques sont enregistrés dans [`commands.txt`](commands.txt) et [`toolchain.txt`](toolchain.txt). Les sorties brutes courantes sont [`contract-current.log`](contract-current.log), [`process-current.log`](process-current.log), [`license-policy-current.log`](license-policy-current.log), [`rustfmt-current.log`](rustfmt-current.log) et [`diff-check-current.log`](diff-check-current.log).

La politique Cargo demeure explicitement sans licence choisie : `license=null`, `license_file=null`, `publish=[]`, `decision=no_license_selected`, `redistribution_rights=not_assessed`. La garde est verte; aucune conclusion juridique ou licence n’est inférée.

## État d’intégration et suite

Le readback d’unités systemd et des ports pertinents figure dans [`runtime-interface-readback.txt`](runtime-interface-readback.txt). Au contrôle ponctuel, `forge-gateway.service` redémarrait automatiquement, `hermes-dashboard.service` était actif mais sans port observé, et aucun service Ollama ni endpoint sur les ports contrôlés n’était disponible. Aucun appel de modèle n’a été fait.

Suite locale utile : prouver l’egress et l’accès aux credentials séparément pour l’exécuteur, le vérificateur et le gate; l’allowlist du client de fusion actuelle est interne au processus et ne remplace pas un contrôle système du réseau. Voir le checkpoint général pour la décision attendue si le sandbox local ne peut pas imposer cette séparation.
