# WEBHOOK-LIVE-MISSION-ADMISSION-007 — run local (24 octobre 2026)

## Périmètre

Le driver `scripts/run_forgejo_live_webhook_mission_admission.py` a été exécuté contre le binaire Forgejo local v17.0.0+gitea-1.22.0, SHA-256 `e5ec437cd6357e8d1b4580cb47ab1daa58a32b33971a63db63446a535d33adb7`. Le checkout source dont le HEAD a été relevé est `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`; les sources étaient modifiées, comme le montrent les 49 entrées de `source_worktree_status`. Le SHA du driver au moment du run était `35a028684c54a16a02c992c60faeff8f511b9c30887bfc29314091eb877113d9`.

L’instance jetable écoutait seulement sur `127.0.0.1:3132`, avec allowlist webhook limitée à la boucle locale. Le dépôt `forge-admin/mission-012-clamp` était privé. Le répertoire d’état privé a été conservé sous `/tmp/forgejo-live-mission-webhook-010-20261024-protection`; son chemin est mentionné dans les reçus, mais ses contenus et les secrets ne sont pas copiés dans ce dossier de preuves.

## Résultat observé

- Forgejo a créé l’issue #1 et émis le vrai événement `issues/opened`, delivery `66df3a24-7ace-4e39-8c24-c045b1c1e549`.
- Le UUID de la tâche Forgejo correspond au delivery du journal Rust; la réponse HTTP lue dans la tâche est 202 et le journal contient exactement une entrée.
- Le CLI a produit une admission versionnée pour la base `9523f7a6a5bd5923181a1d9993f913ba9e7ba9e2`, avec l’exigence `MISSION-012-CLAMP-001` au SHA `3a3c444075c93a3bcfb2bec17d1a2e74ec0051effc2994e6b02ac095eb0dd431` et l’allocation `mission012-live-webhook-001`.
- Le contrôle de branche relu depuis Forgejo exige un approbateur `verifier-bot`, autorise la fusion uniquement par `forge-admin`, refuse les équipes de fusion supplémentaires, bloque les push directs et s’applique aux administrateurs.
- Un credential de lecture incorrect a été refusé avant création de claim/journal. Le rejeu du même delivery a rendu `already_claimed`; les empreintes du claim et du journal sont restées identiques.
- Forgejo a confirmé `active=false` pour le hook avant arrêt. Les processus suivis ont terminé avec le code 0 et aucun listener attendu ne restait sur les ports vérifiés.

## Vérifications et provenance

Le driver et les contrôles consignés sont dans les artefacts de ce dossier. Le manifeste original `SHA256SUMS` couvre ses 10 reçus JSON; `sha256sum -c SHA256SUMS` réussit. La provenance comprend 47 empreintes de sources et les SHA des deux exécutables Rust construits hors ligne et sous Bubblewrap. Les tests enregistrés après le run comptent 10 cas d’admission, 7 cas du garde PR et 6 cas de webhook émis. Les logs de ces contrôles et le présent rapport ont été ajoutés après la génération du manifeste original; ils sont couverts séparément par `SUPPLEMENTAL-SHA256SUMS`.

Le journal de contrôle de licence rapporte `publish=[]`, mais `redistribution_rights=not_assessed`; aucun choix de licence du nouveau crate ni audit des licences des dépendances n’est établi ici.

## Limites

Ce run prouve Forgejo → hook réel → receiver HMAC → admission versionnée et readback de protection. Il n’a pas construit le binaire MCP, exécuté un agent, créé une pull request, exécuté un vérificateur sur le candidat ou fusionné. Il n’a utilisé aucun LLM, AgentOS, RAG ou système de mémoire et ne mesure aucun gain de coût ou de temps. Les services partagent l’UID local; le readback de la tâche webhook lit la base SQLite interne de Forgejo v17. Les détails machine lisibles sont dans `result.json`, `admission.json`, `provenance.json`, `branch-protection-readback.json` et `port-cleanup-readback.json`.
