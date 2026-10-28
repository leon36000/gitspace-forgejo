# CONTINUATION-116 — audit des jointures du parcours composé MISSION-012

Date : 26 octobre 2026. Worktree de produit : `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`, fortement modifié. Ce lot réconcilie les preuves sauvegardées; il ne relance ni Forgejo, ni Cargo, ni l’agent.

## Résultat

La note récente de CONTINUATION-115 disait que l’admission webhook et le candidat stdio-MCP n’étaient pas joints. Cette suite est dépassée : la preuve [MISSION-012-COMPOSE-064](../mission-012-live-agent-webhook-local-adapter-064/local-run.md), déjà enregistrée, couvre le parcours complet dans un seul run local. Le présent vérificateur lit ses reçus existants et ceux de son admission; il ne fait aucun appel de service.

Les jointures vérifiées sont : livraison native `issues/opened` et tâche Forgejo HTTP 202; issue #1 et admission versionnée; même allocation, mission, révision/hash d’exigence et base; hash du fichier candidat produit par l’adaptateur; commit et octets relus par Forgejo; vérification indépendante; commit soumis au gate; trois notifications PR natives réconciliées à l’état courant; SHA du merge; source et tests sur checkout détaché du commit exact. Le checkout de base garde le contre-exemple rouge. L’auditeur refuse aussi quatre altérations en mémoire : ID de livraison différent, hash source du candidat différent, SHA du merge différent, ou source post-merge différente.

Le résultat reproductible est dans [verification.json](verification.json). Les assertions ne prétendent pas certifier Forgejo ni le vérificateur : elles contrôlent les liens entre reçus sauvegardés, et leur source est déjà décrite dans le rapport COMPOSE-064.

## Vérifications

Commande de contrôle des jointures, code 0 :

```sh
python3 evidence/continuation-116-composition-binding-audit-001/verify_composed_binding.py > evidence/continuation-116-composition-binding-audit-001/verification.json
```

Résultat : `pass_read_only_saved_evidence`, six familles de jointures vérifiées et quatre mutations refusées. Adaptateur `local-test-adapter`, `model_invoked=false`, `n=1`.

Les manifestes locaux source des deux dossiers COMPOSE-064 et admission passent `sha256sum -c`; résultat conservé dans [source-manifest-check.log](source-manifest-check.log). Les trois scripts de ce reçu passent `python3 -m py_compile`; aucune source du produit n’a changé dans cet audit. Le dernier append a vérifié 10 fichiers de preuve, ajouté 2 entrées locales, 4 worktree et 4 projet, et préservé les préfixes worktree/projet. `verify_append_only_indexes.py` relit tous les hashes : `local_evidence_files_verified=10`, `parent_binds_current_worktree_index=true`, préfixes worktree/projet préservés. Voir [l’outil](verify_append_only_indexes.py) et [la préparation des préfixes](manifest-prefix-before.json).

Préparation du reçu : la première capture des préfixes a pointé par erreur vers `.runtime/evidence/SHA256SUMS`; elle s’est arrêtée avant toute écriture dans les index. La capture suivante a utilisé l’index du projet parent, puis a enregistré les préfixes exacts. Les essais d’appender ont révélé trois défauts du helper : le premier omettait le nouveau manifeste local dans l’index du worktree; le second tentait aussi de faire référencer le manifeste du worktree par lui-même, une jointure récursive impossible; le troisième ajoutait les nouveaux hashes documentaires au parent sans les ajouter d’abord à l’index du worktree. Le helper place maintenant les hashes documents avant l’append worktree, puis le parent référence le worktree courant. Ces essais ont uniquement ajouté des entrées au bout des index; aucun préfixe ou historique n’a été réécrit. La validation finale doit confirmer les liens courants après le helper corrigé.

## Séparation des affirmations et limites

[FORGEJO-STDIO-MCP-SAME-CANDIDATE-001](../forgejo-stdio-mcp-same-candidate-001/local-run.md) prouve séparément qu’un processus stdio-MCP est encore vivant pendant son propre gate, mais son hash candidat et son hash d’exigence diffèrent de COMPOSE-064. Il n’a pas de livraison native d’issue; aucun hash n’est combiné entre ces runs. Le parcours COMPOSE-064 lie bien le webhook, l’admission, la mission, le candidat, les webhooks PR et le post-merge, mais ne démontre pas la propriété plus stricte où le même enfant MCP issu de cette admission reste vivant pendant le gate.

Pas de relecture du payload/adresse privé ni de secret; aucun service, coût ou accès externe. Cela confirme un parcours fonctionnel `n=1`, pas un gain, une exécution LLM/AgentOS, ni une comparaison Forgejo + agent + CI. Le coût du modèle n’est pas applicable au `local-test-adapter`. Le reçu d’autorité mentionne toujours `external_writer_exclusion=not_provided_by_cooperative_lock`; le risque des writers Forgejo externes demeure.

## Suite

Ne pas rejouer COMPOSE-064. Examiner si la propriété « même enfant MCP admis vivant jusqu’au gate » apporte une couverture de sécurité mesurable; sinon traiter un autre critère local encore ouvert. Le benchmark apparié nécessite toujours un runtime d’agent déjà autorisé et un plafond de dépense fiable explicitement approuvé. Licence et droits de réutilisation de GitSpace non résolus; mission incomplète.
