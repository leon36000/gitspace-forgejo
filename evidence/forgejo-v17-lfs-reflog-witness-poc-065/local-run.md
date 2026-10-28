# POC-065 — reflog comme indice de reprise après déplacement du ref

Date : 25 octobre 2026. Le dépôt Forgejo scratch reste basé sur le commit officiel `0d791421f85ca13a63b665053d81ea765fbbc571` (v17.0.0), mais la reproduction ci-dessous utilise Git directement; elle ne lance pas le serveur Forgejo. Aucun fichier source Forgejo n’est modifié par ce lot.

## Configuration amont et expérience isolée

Dans le code exact du commit, `modules/setting/git.go::loadGitFrom` fixe `core.logAllRefUpdates=true` et `gc.reflogExpire=90`; `modules/git/git.go::syncGitConfig` écrit ces réglages dans le Git config interne, et `modules/git/command.go` lance les commandes avec le HOME interne de Forgejo. L’expérience configure un fichier `GIT_CONFIG_GLOBAL` jetable avec ces deux valeurs, sans lire ni modifier le HOME de l’utilisateur.

Le script `run_reflog_poc.py` crée un dépôt bare et un clone local, pousse successivement le commit de base puis le commit candidat vers `refs/heads/main`, et force ensuite la ref à revenir sur la base. Git 2.43.0 renvoie alors la base comme valeur courante; le candidat n’est plus un ancêtre de la branche, mais son SHA reste dans le reflog `refs/heads/main`, son objet commit est lisible, et `git fsck --unreachable --no-reflogs` le classe inaccessible sans tenir compte du reflog. Sortie : `final.exit=0`.

Base : `d5bd7f05d5e350dcba90ef3fceb101f4fb07a21b`. Candidat temporairement accepté : `70276c775b35bb4582e75b02fd616fea37af092e`. Ref courante finale : base. Le résultat complet, commandes, sorties et versions sont conservés dans ce dossier.

## Lecture pour le protocole LFS

Le reflog peut servir d’indice pour retrouver un commit qui a atteint une ref avant qu’elle soit déplacée. Ce résultat permet de concevoir une reprise basée sur la politique « publication acceptée au moins une fois », même lorsque le ref courant ne contient plus le candidat. Il ne fournit ni ID d’opération, ni autorité CI, ni identité de PR; l’essai n’exerce pas les hooks, les API ou le worker Forgejo, ni les writers concurrents.

La limite de rétention est plus courte que le réglage amont de 90 jours ne le laisse penser : la documentation Git 2.43 indique que les entrées non atteignables depuis le tip courant expirent par défaut après 30 jours via `gc.reflogExpireUnreachable`. Le code de défaut Forgejo v17 ne définit pas cette clé dans `modules/setting/git.go`, mais la section configurable `[git.config]` ou une configuration locale au dépôt peut changer la valeur effective. Le test vérifie `core.logAllRefUpdates` et `gc.reflogExpire`, pas l’expiration effective des entrées non atteignables. Le défaut documentaire n’a pas été accéléré/expérimenté ici. Le reflog n’est donc pas un journal durable à long terme. Une reprise après cette fenêtre doit rester inconclusive/pending ou s’appuyer sur un journal d’opérations durable avec sa propre politique de rétention. Référence primaire : [documentation Git 2.43, `gc.reflogExpireUnreachable`](https://git-scm.com/docs/git-config/2.43.0.html).

## Verrous Forgejo examinés au même commit

`modules/sync/mutex_map.go::MutexMap` contient des `sync.Mutex` en mémoire du processus. `services/repository/branch.go::syncBranchLock` protège `syncSingleBranch` pendant la lecture du ref puis la mise à jour de la ligne `Branch`; son propre commentaire situe la course dans la synchronisation post-receive, après les écritures Git. `services/pull/pull.go::pullWorkingPool` est indexé par ID de PR et porte un commentaire TODO demandant encore un verrou clusterisé. `services/repository/transfer.go::repoWorkingPool` ne protège que le transfert du dépôt. La recherche à l’instantané officiel ne montre donc aucun verrou partagé par tous les writers qui couvre à la fois la mutation de ref et la promotion SQL LFS.

Cette conclusion est limitée aux symboles et chemins examinés, pas à un inventaire exhaustif de plugins ou de tous les call sites internes. Un verrou de promotion nécessiterait une coordination clusterisée, un ordre de verrous et une adaptation cohérente des writers recensés dans l’inventaire; son effort en jours n’est pas mesuré. Aucune modification Forgejo ni décision de fork n’en découle.

## Suite proposée et limites

Avant tout worker : spécifier le sort des stages pending dont le reflog a expiré, vérifier l’application effective des valeurs globales/per-repository, comparer le coût de rétention reflog avec un journal durable, et tester interruption/reprise dans le serveur d’intégration sur un seul writer représentatif. Aucun nettoyage ni expiration forcée n’a été exécuté. Aucun code Forgejo n’est copié au produit, aucun appel LLM ou service externe n’a été fait, et aucune preuve de parcours utilisateur ou de bénéfice comparatif n’est apportée par ce POC.

## Réplication dans le harness Forgejo

Un test ajouté uniquement à `tests/integration/git_push_test.go` du scratch appelle le vrai chemin API `createFileInBranch`, lit le SHA du commit accepté, puis retourne le ref du dépôt sur la base par `git update-ref`. Le test relit la configuration Git effective dans l’environnement de l’application (`core.logAllRefUpdates=true`, `gc.reflogExpire=90`), montre que le commit n’est plus atteignable depuis le ref, mais apparaît dans `reflog show` et reste lisible par `cat-file`. Le test Forgejo ciblé passe 1/1 sous SQLite, Go 1.27.3/linux/arm64 et Git 2.43.0. `gofmt -d` et `git diff --check` ne produisent aucune sortie. Le logger rapporte encore `CreateNotice: context canceled` pendant le teardown; la commande retourne 0.

La première compilation de ce nouveau test a échoué avant exécution parce que trois chaînes typées étaient passées à la liste d’arguments de confiance de `git.NewCommand`; le test utilise maintenant `AddDynamicArguments` et `AddOptionValues`. Le diagnostic et la sortie verte sont archivés. Ce test écrit un fichier ordinaire, pas un objet LFS; il valide le comportement du reflog sur le writer API Forgejo, pas la promotion/récupération LFS. Le ref reset est une commande dans le test, pas un second client concurrent.

Le résultat établit un mécanisme possible d’indice de récupération, pas un témoin durable garanti : le commit exact conserve l’entrée du push après rollback dans ce scénario, tandis que la politique de GC/non-reachable reste bornée et dépend de la configuration. Le code de défaut Forgejo ne définit pas `gc.reflogExpireUnreachable`; la documentation Git 2.43 donne 30 jours par défaut pour les entrées non atteignables. La configuration opérateur ou locale peut modifier cette valeur. Toute reprise après cette fenêtre doit laisser l’opération pending ou consulter un journal distinct; le reflog seul ne peut pas justifier un succès.


Manifeste local : 21 artefacts au niveau racine; le répertoire scratch Git généré est conservé mais validé par le readback idempotent du script plutôt qu’inclus comme arborescence complète dans le manifeste.
