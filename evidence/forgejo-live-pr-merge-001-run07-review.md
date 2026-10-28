# Revue indépendante — FORGEJO-LIVE-PR-MERGE-001 run 07

**Date : 23 octobre 2026.** Revue statique en lecture seule du runner exact et des pièces expurgées du run 07. Aucun test ni service n’a été lancé par le relecteur.

- Runner relu : `scripts/run_forgejo_live_pr_merge_e2e.py`, SHA-256 `c37c30d88ee961767c8d62624c92179cdc1324a7fd2b7405f08260872a1e307a`. Ce hash égale `runner_sha256` dans `forgejo-live-pr-merge-001-run07-trusted-build-provenance.json`.
- Le proxy d’environnement est désactivé pour les appels locaux (`make_api`, ligne 174; health check, ligne 332).
- Le build et l’exécution du candidat utilisent Bubblewrap sans réseau. Le worker candidat ne partage pas le processus du vérificateur ni sa table d’acceptation; le test faux succès est refusé 0/9.
- Le vérificateur lit le contenu à un SHA de tête immuable, teste ces octets, puis relit head/base avant de signer. Le préflight est relu avant la fusion et le POST transmet `head_commit_id`.
- Le code assert les champs de protection des approbateurs et mergeurs et leurs listes exactes. Le merge par l’utilisateur hors liste est attendu refusé par Forgejo et la PR est relue toujours ouverte.
- Le contrôle rouge exige les cinq échecs, quatre réussites et le décompte `4 passed; 5 failed`; le read-back run 07 correspond.

**Résultat de cette revue :** aucun défaut matériel trouvé dans ces six catégories pour le runner correspondant au hash indiqué. Il s’agit d’une analyse de code/evidence, pas d’une nouvelle exécution. Le parcours vérifié reste un adaptateur déterministe sans LLM; neuf cas ne démontrent pas une correction générale. Les limites notées dans le rapport principal restent applicables : autorité de fusion détenue par le driver de test, absence de CAS atomique sur la base, pas de limite mémoire cgroup ni d’isolation contre un adversaire du même UID, et fenêtres TOCTOU possibles pendant le staging.
