# CONTINUATION-245 — vérification du gate indépendant pendant la vie du MCP

Date : 27 octobre 2026. Worktree préexistant `.runtime/webhook-005-workspace`; révision de base `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`. Aucun reset, commit, publication, service de production ni appel à un modèle externe.

## Résultat

Le run composé sur l’instance locale Forgejo v17.0.0 a terminé `merged_in_private_sandbox`. Il a accepté la livraison webhook `36dd0a39-ffc9-4f6d-a196-080e4cb782fd`, lié l’admission au candidat `087db2429250b3b92e35bc3bf2e9531f75eb14da`, ouvert la PR locale #2, évalué le candidat par un gate indépendant et fusionné le commit `ecd6e5fb3c3b8f07c29454815ca1befa8e637664`. La tête `main` relue est exactement ce commit; la PR et l’issue sont `closed`. Les trois notifications PR (ouverte, revue approuvée, fermée) ont été réconciliées par readback d’état courant; la notification d’approbation n’a pas été traitée comme preuve d’acceptation.

La tâche de la fixture versionnée `MISSION-012-CLAMP-001` demande de corriger la borne supérieure de `clamp_to_range`, de préserver borne inférieure et valeurs dans l’intervalle, de ne modifier que `src/lib.rs`, et de tester d’abord la base inchangée. La base rouge échoue sur `values_outside_the_range_clamp_to_the_nearest_boundary` (valeur observée `-2`, attendue `10`; code de sortie 101). Le candidat ne change que `src/lib.rs`, avec l’empreinte `173304ad7d32aabc1e8e1ee41eabda3398a8690798e68f906bcebc73cbcd6d15`; les deux tests d’acceptation passent dans la vérification indépendante. Une nouvelle exécution avant fusion passe également, puis l’acceptation exécutée après fusion sur le commit exact `ecd6e5fb3c3b8f07c29454815ca1befa8e637664` passe `2/2` (`exit_code=0`). Le run démontre donc un échec actionnable et une correction vérifiée sur cette petite fixture.

## Barrière de vie du processus

Le reçu `8a3d69512e1f9d438ac89dd988e3e1658822d73a6ccc473a5213e553bef434be` atteste `observed_while_mcp_host_alive=true`. Il lie le même enfant MCP admis par PID/start-time, superviseur et empreinte d’exécutable (`8904b872f4624175ff51236cb63f51223fdea3103f2ea8f385de4d2bbc4fd0a1`) au hash source du candidat `173304ad7d32aabc1e8e1ee41eabda3398a8690798e68f906bcebc73cbcd6d15`, à la base `f7d7f5dabf7a258de8204fa682f8477414de1eac`, aux exigences `3a3c444075c93a3bcfb2bec17d1a2e74ec0051effc2994e6b02ac095eb0dd431` et à la politique `e23907b5bd6b2c0f342c88ddcfa9f9a9be98065adcd24f9f7299dc796b5739eb`. Pendant que cet enfant demeurait vivant, le vérificateur distinct a lancé `cargo test --offline --locked --test acceptance` dans une vue candidate en lecture seule, avec namespaces Bubblewrap et sans home hôte ni credential Forgejo; résultat `2 passed; 0 failed`. Le composeur vérifie ensuite que le reçu sur disque, son hash, l’empreinte candidate finale et celle de l’exécutable MCP correspondent au résultat. Après le gate, le superviseur libère l’arrêt, l’étape agent se termine, puis le stade PR/merge démarre. **Le reçu ne prétend pas que le MCP est resté vivant pendant la fusion.**

La sandbox de l’étape agent rapporte réseau non connecté, aucun credential visible, aucun fichier runtime privé visible, routes API/Web/receive-pack bloquées et zéro connexion à ces routes observée par le superviseur. Cette preuve concerne le runner local et ses sondes; elle n’équivaut pas à un audit de toutes les capacités possibles de l’hôte.

## Autorisation et lecture après fusion

La décision de merge utilise `forgejo-reconcile-local-rust`; une tentative avant revue et une tentative par le reviewer non autorisé ont toutes deux été refusées en HTTP 405. Le verdict indépendant est `pass`, l’arbre du merge correspond à l’arbre candidat et le post-test a été exécuté sur le commit relu. Le binaire Forgejo annonce `17.0.0+gitea-1.22.0`, SHA-256 `e5ec437cd6357e8d1b4580cb47ab1daa58a32b33971a63db63446a535d33adb7`; le commit exact de construction du binaire n’est pas établi. L’origine est loopback et le dépôt est privé dans ce bac à sable local.

## Adaptateur et coût

`agent_adapter=local-test-adapter`, `model_invoked_during_run=false`; le champ de coût rapporté par le harnais vaut `0.0` USD. Aucun LLM, Claude, AgentOS ou modèle n’a été utilisé; les tokens ne s’appliquent pas. Cela vérifie l’interface du parcours et le gate, pas le comportement d’un agent génératif ni son coût fournisseur.

## Échec du premier lancement

La première commande est sortie avec code 1 avant les étapes de mission : le harnais exigeait un nom de dossier sous la forme `mission-012-live-agent-webhook-<run>`. Aucun state dir n’a été créé (`/tmp/mission012-cont245-state-001` absent). Ce refus est un problème de nommage/préflight du harnais, pas un échec produit; les commandes, sortie et snapshot de ce premier essai sont conservés ici. Le second lancement utilise le nom admis `mission-012-live-agent-webhook-cont245-gate-002` et a terminé avec code 0.

## Preuves et limites

Le reçu de snapshot contient 114 fichiers et son manifeste interne SHA-256 a été revérifié : toutes les empreintes et tailles concordent; les 114 hash enregistrés dans `composed-run.json` concordent aussi avec le worktree courant. Le snapshot zip a le SHA-256 `aac8129396be9fd236c98211015aef65c73ed66e78d84ef753f97ac169e3be22`. Les JSON et sorties du composeur restent dans [le dossier de run](../mission-012-live-agent-webhook-cont245-gate-002/).

C’est `n=1` sur une tâche déterministe et une instance locale. Ce n’est pas une comparaison appariée Forgejo+agent+CI, ne mesure pas exigences satisfaites sur un corpus, régressions échappées, interventions, tokens, coût ou temps total, et n’établit aucun gain général. Cela ne supprime pas les limites d’atomicité de Forgejo face à tous les writers, ni les réserves juridiques sur les licences. Le projet reste incomplet.
