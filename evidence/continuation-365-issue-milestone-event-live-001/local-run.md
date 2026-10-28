# CONTINUATION-365 — événements milestone d’issue Forgejo live

Date : 28 octobre 2026. Worktree : `local/WEBHOOK-005-canonical-links`, HEAD `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`. Aucun commit, push, accès courriel/GitLab ou changement de production.

## Lacune et code local

Le reconciler projetait déjà le milestone depuis l’API courante et les contrats testaient les états `unknown`, `unassigned` et `assigned`, mais le runner Forgejo live n’exerçait pas les événements réellement produits lors de l’affectation et du retrait d’un milestone. Ce lot modifie uniquement le runner Python local et ses tests :

- `--change-issue-milestones` crée un milestone jetable, l’affecte à l’issue, puis le retire avec l’API.
- Le parseur accepte `milestoned` et `demilestoned`, exige le champ `issue.milestone`, valide ID/titre/état et distingue un `null` explicite d’un champ absent.
- Le scénario lie exactement une livraison et une tâche Forgejo par transition, compare le snapshot `milestoned` au GET API, valide `null` après retrait et réconcilie les deux événements historiques vers l’état API courant non assigné.
- Les offsets des événements d’issue et de PR incluent les deux nouvelles livraisons.

Aucun code Rust, Forgejo ou GitSpace n’a été modifié/copier dans ce lot. Le code de réconciliation utilisé est le checkout déjà modifié du produit local; les digests exacts de ses binaires et sources sont dans `forgejo-run-001/provenance.json`.

## Sources primaires Forgejo v17

Le scratch `/tmp/forgejo-v17-base-cas-poc` est au commit officiel Forgejo v17.0.0 `0d791421f85ca13a63b665053d81ea765fbbc571`; `git diff --exit-code HEAD -- services/webhook/notifier.go services/issue/milestone.go modules/structs/issue.go modules/structs/issue_milestone.go modules/webhook/type.go routers/api/v1/repo/issue.go routers/api/v1/repo/milestone.go` retourne 0 dans ce scratch, bien que le scratch complet contienne d’autres POC. La commande et son résultat sont dans `forgejo-source-diff-check.txt`. Inspection ciblée de source seulement; aucun test Go amont ni build de Forgejo n’a été exécuté dans ce lot.

| Fichier/symbole | Conclusion vérifiée | SHA-256 du fichier |
| --- | --- | --- |
| `services/webhook/notifier.go::IssueChangeMilestone` (561–599) | `MilestoneID > 0` choisit `milestoned`, zéro choisit `demilestoned`; l’issue est envoyée par `HookEventIssueMilestone`. | `a67cca36f92104efe0feabf0c62e668e7543b979e5a7440e713ce93dd490004f` |
| `services/issue/milestone.go::ChangeMilestoneAssign` (91–106) | Met à jour l’assignation puis notifie le service webhook. | `0b7e965caff0c5de0660b61929e92b42d75fbe1d116352a9d4b1b5b2fd6475d4` |
| `modules/structs/issue.go::Issue.Milestone`, `EditIssueOption.Milestone` (59, 113–124) | L’issue expose `milestone` sans `omitempty`; l’édition prend un `*int64` documenté comme ID. Un pointeur non nul vers zéro représente le retrait exercé ici. | `e6cb7890f1d7045b2423465d1e452b6f91c4d71c746957aefc9bade286d10006` |
| `modules/structs/issue_milestone.go::Milestone` (11–26) et `routers/api/v1/repo/milestone.go::CreateMilestone` (129–180) | Le readback expose `id`, `title`, `state`; la route de création répond 201. | `a204a417bd519072ad5ac0e64fb804a0c2fc1fb4a0b39d71ee6d690d065877b8`; `d542f990464a5532e58b0a979ac1b81347a2c54a6c1fec3c25f5e9e1e11e1ab1` |
| `routers/api/v1/repo/issue.go::EditIssue` (788–960, traitement milestone vers 916–923) | L’ID reçu déclenche `ChangeMilestoneAssign`; le handler répond 201. | `c88041dc09cdc1097549f00d25fc5a33aefee443c21b5be2ce2e4198b4dcf576` |
| `modules/webhook/type.go::HookEventType.Event` (56–70) | Le type spécialisé `issue_milestone` utilise l’en-tête d’événement HTTP `issues`; la tâche interne garde `issue_milestone`. | `ca6a94e1574be77bc4f5da1ee01a3e59147a148342b958577d12544dbd6d820c` |

Cette revue confirme les call sites de l’action et du readback pertinents seulement; elle ne couvre pas toute la surface webhook/API de Forgejo ni ses dépendances/licences.

## Test d’abord, puis vérifications

Avant l’implémentation, le test CLI échouait parce que `--change-issue-milestones` manquait; les tests parser échouaient parce que les actions étaient rejetées, et le test clear parce que le validateur n’existait pas. Après l’implémentation, le sous-ensemble ciblé passe 5/5 (`focused-tests.log`) et `PYTHONDONTWRITEBYTECODE=1 PYTHONPATH=tests python3 -m unittest -q test_forgejo_emitted_webhook_e2e` passe **160/160** (`python-unittest.log`). Les logs de syntaxe, git-diff et Cargo sont aussi conservés; `run-source-hash-check.json` confirme que les empreintes du runner et du test capturées dans la provenance live correspondent aux fichiers présents.

Commande live exécutée depuis la racine du worktree :

```text
PYTHONDONTWRITEBYTECODE=1 python3 scripts/run_forgejo_live_webhook_reconcile_emitted.py --forgejo-binary /tmp/forgejo-17.0.0 --repo-root /home/pc6/Documents/Codex/2026-10-21-mission-construire-une-forge-con-ue/.runtime/webhook-005-workspace --state-dir /tmp/forgejo-cont-365-issue-milestone-state-001 --evidence-dir /home/pc6/Documents/Codex/2026-10-21-mission-construire-une-forge-con-ue/.runtime/webhook-005-workspace/evidence/continuation-365-issue-milestone-event-live-001/forgejo-run-001 --change-issue-milestones
```

Le processus sort avec code 0 et `result.json` indique `verdict=passed`. Binaire : `17.0.0+gitea-1.22.0`, construit avec Go 1.26.9, SHA-256 `e5ec437cd6357e8d1b4580cb47ab1daa58a32b33971a63db63446a535d33adb7`; son commit de build exact n’est pas établi. Le hook lit de retour contient `issue_milestone` et est désactivé avant arrêt. Les neuf tâches enregistrées ont toutes un statut livré/réussi et HTTP 202.

Le POST de création et les deux PATCH renvoient 201. Forgejo émet exactement deux événements distincts : `milestoned`, séquence 2, avec `{id:1,title:"agent-forge-e2e-live-issue-milestone",state:"open"}`, puis `demilestoned`, séquence 3, avec `milestone:null`. Le GET après retrait confirme aussi `milestone:null`. Chaque livraison se réconcilie en `current_state_read`, avec un GET, source `current_forgejo_resource_api`, observation `unassigned` et valeur `null`. Le readback de nettoyage confirme processus arrêtés et ports loopback attendus absents.

Les 18 fichiers du run vérifient leur `SHA256SUMS`. Un premier contrôle a été lancé depuis la racine au lieu du répertoire du run et n’a pas trouvé les chemins relatifs; le contrôle a été relancé depuis le bon répertoire et passe intégralement. Le journal final est `forgejo-run-001-manifest-check.log`.

Cargo ne change pas de politique : tests de garde 12/12; `verify_cargo_license_state.py` rapporte `license=null`, `license_file=null`, `publish=[]`, `no_license_selected`, `redistribution_rights=not_assessed`, `NOASSERTION`. Versions locales : Python 3.12.3, Cargo 1.95.0, rustc 1.95.0.

## Limites et suite

Un parcours local jetable (`n=1`) démontre l’émission, le binding et le readback des deux actions sur ce binaire; il ne prouve pas un gain statistique, le comportement de toutes les versions Forgejo, l’exécution d’un LLM/AgentOS, une PR de code ou une fusion contrôlée. L’absence de commit de build du binaire limite le rattachement entre son comportement et les sources officielles auditées. Aucun droit de redistribution ou licence produit n’est établi.

Prochaine tranche locale utile : tester à la frontière processus/réseau qu’un agent lancé isolément ne peut joindre les routes Forgejo API/Web/receive-pack ni lire un credential de fusion, alors que seul le gate conserve les requêtes autorisées. Le projet reste incomplet; le benchmark contrôlé et les autres refus requis restent ouverts.
