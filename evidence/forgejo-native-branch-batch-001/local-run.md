# Réconciliation batch des événements natifs `create` et `delete` — 25 octobre 2026

## Résultat

Une instance jetable Forgejo v17.0.0 a émis deux webhooks pour la même branche privée : `create` (delivery `97e16b07-84c6-427b-8d3b-d4c2aba8b08d`) puis `delete` (`176cbe52-1812-49b5-970a-102af38d585e`). Le commit du payload `create` (`bf207263ab2a52c3da58a1b4353a0da7fd341b65`) est devenu obsolète après un push ultérieur. La branche a été supprimée, recréée avant réconciliation, et l'API Forgejo a confirmé comme tête courante `840ebf89a9065ffbf8bc7ab49ec1013902bbc16f`.

Le runner a passé les deux delivery IDs dans une seule invocation de `forgejo-reconcile-local`. Le résultat batch schema 5 lie les deux résultats à leurs delivery IDs respectifs; chacun est `current_state_read`, nomme la même branche et la même tête API. Le compteur du reconciler rapporte `upstream_read_attempts=1`. C'est le compteur applicatif du CLI; ce run ne contient pas de capture indépendante du trafic HTTP. Le journal webhook est identique avant et après la réconciliation.

La tâche Forgejo a reçu HTTP 202 pour `create` et `delete`. Le hook a été désactivé avec readback confirmé; Forgejo et le receiver sont arrêtés avec code 0 et sans listener résiduel. Le credential read-only est transmis par FD hérité privé, le runner confirme qu'il a été consommé, et aucune valeur de secret ou credential n'est enregistrée.

## Reproduction et provenance

Commande du parcours :

```text
python3 scripts/run_forgejo_native_branch_delete_webhook_reconcile.py --forgejo-binary /tmp/forgejo-17.0.0 --state-dir /tmp/forgejo-native-branch-batch-001 --evidence-dir evidence/forgejo-native-branch-batch-001 --include-create
```

Le runner a construit le receiver et le CLI depuis le worktree courant avec Cargo `1.95.0`, `--offline --locked`, sous Bubblewrap `0.9.0`, avec réseau/PID isolés et source, registre et sysroot en lecture seule. Les empreintes du binaire Forgejo (`e5ec437cd6357e8d1b4580cb47ab1daa58a32b33971a63db63446a535d33adb7`), des binaires Rust et des sources testées figurent dans [result.json](result.json); le log de compilation est dans [cargo-build.log](cargo-build.log). Le code Forgejo source officiel consulté pour v17.0.0 est au commit `0d791421f85ca13a63b665053d81ea765fbbc571`; le commit ayant produit ce binaire local n'est pas établi.

Vérifications : le validateur batch ciblé passe 4/4 ([journal](focused-validator-tests.log)), le module Python complet passe 67/67 ([journal](python-driver-tests.log)), et `py_compile`, `cargo fmt --all -- --check` ainsi que `git diff --check` passent ([journal](verification.log)). Le parcours live reconstruit les deux binaires Rust puis exécute le CLI réel. Aucun changement Rust n'a été fait dans ce lot; `cargo test --all-targets` n'a pas été relancé ici. Le manifeste local recouvre tous les artefacts de ce rapport.

## Portée et limites

Cette preuve couvre un dépôt et une branche jetables, l'ordre `create` puis `delete`, la liaison de résultats batch et une seule lecture upstream comptée par le reconciler. Elle ne teste pas le cas branche absente, les événements désordonnés, les retries, un fournisseur LLM/AgentOS, une fusion PR, ni une comparaison de productivité. Le binaire Forgejo n'est pas lié à un commit de build connu. Ce résultat n'établit pas de gain général ni une garantie de sécurité complète.

Les artefacts JSON, le build log et leurs empreintes locales sont dans ce dossier; [result.json](result.json) contient les IDs et readbacks complets.
