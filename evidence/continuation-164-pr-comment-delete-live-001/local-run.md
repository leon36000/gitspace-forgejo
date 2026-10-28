# CONTINUATION-164 — suppression live d’un commentaire de PR

Date : 26 octobre 2026. Worktree : `/home/pc6/Documents/Codex/2026-10-21-mission-construire-une-forge-con-ue/.runtime/webhook-005-workspace`; HEAD de base `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`. Le worktree était déjà fortement modifié; aucun reset, commit, changement de branche, publication ou modification de production.

## Question et résultat

CONTINUATION-163 prouvait le readback d’un commentaire PR live créé, mais pas le cas natif `deleted`. Le runner existant a reçu un mode isolé `--delete-pull-request-comment` : il conserve le webhook `created` émis durant le setup, supprime le commentaire par l’API, lit à nouveau son endpoint et attend le webhook `deleted`. Il relance ensuite les deux deliveries par le vrai CLI Rust après la suppression.

Sur une nouvelle instance Forgejo locale v17, la création du commentaire ID `4` a produit `issue_comment/created` (séquence 6); DELETE a répondu 204, puis GET `/issues/comments/4` a répondu 404. Forgejo a émis un `issue_comment/deleted` distinct (séquence 7). Le CLI a gardé l’ancien `created` `inconclusive/resource_not_found`, `retryable=false`, code de sortie 1, après 2 lectures; il n’a pas présenté ce 404 comme une absence canonique. Pour le `deleted`, le parent PR #2 a été lu avec succès et le CLI a retourné `current_state_read`, 2 lectures et `comment={id:4,state:"absent",body:null,updated_at:null,html_url:null}`. Le readback direct séparé par le propriétaire est consigné comme `{comment_id:4,http_status:404,verdict:"absent"}`. Les tâches Forgejo lient les deux UUID aux événements `pull_request_comment`, HTTP 202. Le run s’est achevé `passed`; le hook a été désactivé et relu, Forgejo et receiver se sont arrêtés et aucun listener TCP attendu ne restait.

Le parcours de création n’est qu’une précondition de la suppression; le résultat inédit de ce lot est l’observation live `deleted` et la différence de verdict pour l’ancien `created` après 404. Aucune logique Rust de réconciliation n’a changé dans ce lot; le changement est le validateur de reçu et le scénario d’intégration local.

## Vérifications et provenance

- TDD : le test nouveau a d’abord échoué parce que `validate_pull_request_comment_reconciliation` n’acceptait pas le statut HTTP du readback. Après correction, classe ciblée : 6/6; suite `tests/test_forgejo_emitted_webhook_e2e.py` : 118/118; `py_compile` : succès.
- Le runner a construit `forgejo-webhook-local` et `forgejo-reconcile-local` avec Cargo `--offline --locked` sous Bubblewrap 0.9.0, `--unshare-net --unshare-pid`, environnement effacé et source/registry/sysroot lus seuls. Versions de build : Cargo 1.95.0, rustc 1.95.0. Voir `cargo-build.log` et `provenance.json`.
- Cargo metadata et le garde dédié conservent `license=null`, `license_file=null`, `publish=[]`, `no_license_selected`, `redistribution_rights=not_assessed`, `NOASSERTION`. La licence du produit n’est pas choisie; les droits de redistribution ne sont pas évalués.
- Binaire Forgejo : `17.0.0+gitea-1.22.0`, Go 1.26.9, SHA-256 `e5ec437cd6357e8d1b4580cb47ab1daa58a32b33971a63db63446a535d33adb7`. Cette version et cette empreinte n’établissent pas le commit exact de build. Le dépôt source officiel audité reste le tag v17.0.0 / commit `0d791421f85ca13a63b665053d81ea765fbbc571`; aucun code Forgejo n’a été copié ou modifié.
- Les scripts, sorties structurées, empreintes, commandes et vérifications de manifestes sont dans ce dossier; voir `source-sha256.txt`, `run-command.txt` et les JSON de reconciliation. L’instance a utilisé le port loopback 3130 et un state privé jetable `/tmp/forgejo-pr-comment-delete-live-164`, laissé en place.
- Les manifestes sont append-only et gardent les anciennes empreintes si un log local est corrigé : `verify_append_only_indexes.py` vérifie la dernière empreinte de chaque entrée, les 32 fichiers locaux, les préfixes worktree/projet et le lien parent.

## Portée et limites

Un seul run sur une instance locale jetable (`n=1`). Forgejo, receiver et CLI tournent sous le même utilisateur hôte; cela n’est pas une preuve d’isolation contre un processus hostile de même UID. Les GET de la PR, du commentaire et le readback externe sont séquentiels et non atomiques. La réponse 404 ne prouve pas l’absence sur tous les nœuds ou dans toutes les conditions d’autorisation. Aucun LLM, AgentOS, RAG, mémoire, prompt injection, merge, CI conventionnelle, benchmark, mesure de coût ou comportement de production n’a été exécuté. Aucun appel payant ou service externe n’a été utilisé.

## Suite

Ne pas répéter les parcours de commentaires PR déjà prouvés. Continuer avec une lacune non couverte dans la matrice; l’AgentOS/runtime réel et la comparaison contrôlée restent ouverts tant qu’aucune intégration locale utilisable et aucun plafond financier vérifiable n’existent. Le projet demeure incomplet.
