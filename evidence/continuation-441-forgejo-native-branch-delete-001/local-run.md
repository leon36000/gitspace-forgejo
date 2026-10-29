# CONTINUATION-441 — suppression native de branche et réconciliation courante

Date : 28 octobre 2026. Exécution sur le worktree PC6 `local/WEBHOOK-005-canonical-links`, HEAD `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`.

## Résultat observé

Le runner dédié a démarré une instance Forgejo locale jetable, créé un dépôt privé et la branche `agent/cont440-delete-check`, puis supprimé cette branche par Git. Forgejo a émis son événement natif `delete`; sa delivery UUID a été acquittée par le receiver local HTTP **202**. Le vrai `forgejo-reconcile-local`, compilé hors réseau avec `cargo build --offline --locked` dans Bubblewrap, a retourné `current_state_read`, `canonical_state.state=absent` et `upstream_read_attempts=3`. La référence distante était absente après le push de suppression, et le journal est resté inchangé pendant la réconciliation. Le résultat global du runner est `passed`.

Le résultat donne le nombre de lectures et le snapshot final; il ne conserve pas une trace individuelle complète de chaque réponse HTTP. L’affirmation se limite donc au résultat du CLI et au chemin de code vérifié, pas à un journal HTTP indépendant pour les trois statuts.

## Fermeture locale

Le hook a été désactivé puis relu comme inactif. Le processus Forgejo et le receiver se sont arrêtés avec code 0; `cleanup_failure=null`, les listeners post-arrêt sont vides. Le répertoire `/tmp/agent-forge-cont440-state` est conservé, mode `0700`, pour éviter toute suppression implicite; il contient l’état local privé de la fixture. Aucun credential n’est enregistré dans les reçus. Aucun service externe ou de production n’a été contacté.

Les preuves détaillées et empreintes sont dans `native-run/`; le binaire Forgejo a le SHA-256 `e5ec437cd6357e8d1b4580cb47ab1daa58a32b33971a63db63446a535d33adb7` et annonce **forgejo version 17.0.0+gitea-1.22.0 (release name 17.0.0) built with GNU Make 4.4.1, go1.26.9 : bindata, timetzdata, sqlite, sqlite_unlock_notify**. Ce binaire correspond au hash du reçu natif CONTINUATION-433, mais son commit exact de build n’est pas établi. Le code Forgejo officiel audité demeure la référence de source distincte au commit `0d791421f85ca13a63b665053d81ea765fbbc571`.

## Limites

Un événement, une branche et une instance jetable (`n=1`). Les 404 restent ambigus si l’accès au dépôt est masqué; le test couvre une lecture réussie de l’instance locale. Il ne teste pas la recréation de branche après l’événement, une base concurrente, une PR, une fusion, AgentOS/LLM, l’interface après fusion ou un benchmark. Il établit une compatibilité native locale supplémentaire, pas la correction générale ni un gain de productivité.
