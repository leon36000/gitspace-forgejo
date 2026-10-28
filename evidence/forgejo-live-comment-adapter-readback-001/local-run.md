# FORGEJO-LIVE-COMMENT-ADAPTER-READBACK-001 — readback canonique des transitions de commentaire

Date : 26 octobre 2026. Parcours exécuté dans une instance Forgejo jetable locale, n=1.

## Tranche réalisée

Le contrat Rust issue_comment distingue le commentaire de l'issue parente et lit l'issue puis GET /repos/{owner}/{repo}/issues/comments/{id}. Le runner E2E réconcilie chaque livraison après réception et pendant que l'état correspondant est encore disponible. Il vérifie la ressource, l'identité du commentaire, les deux lectures amont, l'URL canonique, le corps lu via l'API, l'étiquette untrusted_user_content et l'absence canonique après suppression.

L'instance locale a créé, édité puis supprimé le même commentaire #6 de l'issue #1. Forgejo a émis created, edited, deleted aux séquences 8–10; le receiver a reçu chaque livraison en HTTP 202. Pour chaque réconciliation, le binaire local a retourné current_state_read, le schéma 5, deux lectures amont et le bon ID. Les readbacks créés/édités contiennent respectivement le corps initial et le corps modifié; après suppression, l'adaptateur retourne state=absent et aucun corps, date ni URL. Le GET séparé après le DELETE retourne également 404. Le résumé machine vérifié est dans canonical-readback-summary.json; les événements et résultats bruts restent dans les reçus voisins.

Cela ferme le trou du précédent rapport de lecture canonique : les anciens parcours live ne lisaient que l'issue parente; les tests de l'adaptateur commenté utilisaient une fixture HTTP. Aucun code de production n'a été ajouté dans ce lot; le changement est dans le validateur et le runner local pour qu'il exerce effectivement l'interface Rust existante.

## Provenance et contrôles

- Worktree : base Git f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0, branche locale sale; les empreintes post-changement des scripts/tests/Rust sont dans source-hashes.txt.
- Forgejo annonce 17.0.0+gitea-1.22.0, SHA-256 e5ec437cd6357e8d1b4580cb47ab1daa58a32b33971a63db63446a535d33adb7. Le commit source ayant construit ce binaire n'est pas établi.
- python3 -m unittest tests.test_forgejo_emitted_webhook_e2e : 98 réussis.
- python3 -m py_compile scripts/run_forgejo_live_webhook_reconcile_emitted.py tests/test_forgejo_emitted_webhook_e2e.py : réussite.
- Le parcours compose le vrai Forgejo local, le receiver HMAC et les binaires Rust construits hors ligne; succès pour les trois readbacks et le nettoyage. Les listeners attendus sont absents après arrêt.
- cargo fmt --all -- --check, git diff --check : réussis.
- Relecture Cargo fraîche : license=null, license_file=null, publish=[]; métadonnées no_license_selected, redistribution_rights=not_assessed, marqueur d'export NOASSERTION. Le vérificateur de politique et ses 9 tests passent. Aucun choix de licence n'a été fait.

Le premier test TDD du validateur a échoué sur l'assertion attendue parce que le contrôle de readback canonique du runner n'existait pas; la version implémentée et les vérifications vertes sont consignées dans le transcript de travail et les fichiers de résultats ci-joints. La validation de corps, identité et statut est aussi testée négativement par des mutations de l'objet JSON passé au validateur. Après l'E2E, un test redondant de disponibilité a été retiré et un cas négatif de corps périmé a été ajouté; le code du runner n'a pas changé après l'E2E, et la suite Python finale passe 98/98. provenance.json conserve le hash historique exact de la suite présent au moment du run; source-hashes.txt donne les hashes finaux.

## Limites

Une instance et un commentaire synthétique, sans Forgejo de production. Le binaire Forgejo n'est pas attesté comme issu du commit source audité. Pas de LLM, AgentOS, RAG ou mémoire; aucun corps utilisateur ou secret n'a été transmis à un service externe. body_trust est une frontière de donnée, pas une sanitation; l'intégration avec un consommateur agent demeure non démontrée. Cette tranche ne mesure ni comparaison conventionnelle ni gain général.
