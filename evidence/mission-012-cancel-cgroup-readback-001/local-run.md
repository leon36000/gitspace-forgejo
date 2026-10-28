# MISSION-012-CANCEL-CGROUP-READBACK-001 — readback robuste après interruption

Date : 26 octobre 2026. Worktree basé sur `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`, fortement modifié. Ce lot modifie seulement un test d’intégration Rust; aucun code runtime ou Forgejo n’est changé.

## Lacune et correction

Un run complet antérieur avait échoué après le délai de nettoyage du scope verifier : le diagnostic a relu un PID listé dans `/proc`, mais ce PID avait déjà disparu. Le test mesurait `cgroup.procs` en balayant `/proc` puis supposait que l’échantillon était encore actuel à l’assertion suivante. Une course de lecture est une explication plausible, pas une cause entièrement reproduite.

Le readback vérifie maintenant à nouveau, pour chaque PID candidat, que `/proc/<pid>/stat` existe encore, que le processus n’est pas en état zombie/exiting (`Z`/`X`) et que `/proc/<pid>/cgroup` indique toujours le scope attendu (ou un descendant). Après le délai de polling, le test fait encore un readback juste avant l’assertion. Un test du helper couvre un membre vivant, un sous-cgroup, un PID disparu, `Z`/`X` et un processus d’un autre scope. La capacité de tuer le scope et ses limites restent inchangées.

## Vérifications

- Test-first : avant l’implémentation du helper, le test ciblé échoue à la compilation car `live_process_in_cgroup` n’existe pas (`red.log`).
- Helper ciblé : 1 réussi, 0 échec.
- Test réel `mcp_supervisor_cancellation_terminates_the_in_flight_verifier_scope` : quatre exécutions séquentielles réussies, chacune avec le vrai host MCP, le vrai superviseur, Bubblewrap/systemd et un verifier suspendu.
- Target `mission_012_mcp_lifecycle` : 29 réussis, 0 échec, 1 ignoré.
- `cargo fmt --all -- --check` et `git diff --check` : code 0 après correction de formatage.
- Les commandes exactes, outils et hashes sont dans les fichiers voisins.
- Le manifeste local reste append-only. Il conserve les hashes précédents des fichiers mis à jour après la dernière relecture; la validation vérifie la dernière ligne de chaque chemin distinct, sans réécrire l’historique.

## Limites

Trois répétitions réussies réduisent l’incertitude sur cette observation sans exclure une autre intermittence. Aucun test complet `--all-targets` n’a été relancé dans ce lot. Le test et sa lecture `/proc` ne sont établis que sur Linux avec systemd/cgroup v2 de cet hôte; ils ne couvrent ni panne noyau/alimentation, ni une autre implémentation du superviseur, ni un refus simultané de `cgroup.kill` et du bus systemd. Le test prouve son scénario local, pas une isolation contre root ou un noyau compromis. Aucun LLM, AgentOS, dépense, service externe, secret, publication ou changement de production n’a été utilisé. Le benchmark contrôlé et le parcours LLM répété restent ouverts.
