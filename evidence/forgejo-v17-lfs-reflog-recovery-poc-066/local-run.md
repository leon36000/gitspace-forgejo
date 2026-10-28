# POC-066 — reprise LFS à partir de la reflog de la ref cible

Date : 25 octobre 2026. Cible d’audit : Forgejo officiel v17.0.0, commit `0d791421f85ca13a63b665053d81ea765fbbc571`. Les modifications testées sont uniquement dans le scratch `/tmp/forgejo-v17-base-cas-poc`; aucun code Forgejo n’a été copié dans le produit, publié ou déployé.

## Hypothèse et résultat

POC-065 a montré que le commit candidat d’une mise à jour de ref acceptée peut rester dans la reflog de la ref après un retour de la branche vers sa base. Le premier test rouge de ce lot a confirmé que le récupérateur LFS existant ne consultait que les refs courantes : il laissait alors pending une opération dont le candidat avait été publié puis retiré (`red-e2e.exit=1`).

Le patch expérimental ajoute `candidateAppearedInTargetRefReflog` à `services/pull/lfs_recovery.go`. La reprise n’accepte que le SHA candidat exact dans la reflog de la ref cible, et uniquement si cette ref existe encore. Une erreur de lecture échoue fermé et laisse l’association pending. Le même contrôle est utilisé par le balayage non paginé et la page keyset; le cas d’une ref voisine `main-shadow` ne suffit pas. Ce témoin signifie « observé dans l’historique de la ref », pas « joignable actuellement ».

Résultats frais : sept tests de récupération du paquet `./services/pull` passent, y compris le chemin paginé, les reprises existantes, le candidat absent, le refus d’une ref sans rapport et l’expiration forcée de la reflog. Après expiration, la page laisse l’opération pending et l’objet actif absent. Le test d’intégration Forgejo passe 1/1 sous SQLite : l’API fichiers reçoit `.gitattributes` et un fichier `.bin` LFS; un seam déplace `main` à la base après l’acceptation du commit, avant la promotion post-receive. L’objet reste pending et masqué (`GET` 404); la récupération lit la reflog de `main`, promeut l’association puis le GET LFS renvoie HTTP 200 avec les octets originaux. `main` reste à la base pendant cette reprise.

Les empreintes des quatre sources pertinentes passent `sha256sum -c`; `gofmt -d`, `git diff --check` sur le fichier d’intégration et un contrôle de fin de ligne sur trois fichiers passent aussi.

Le journal d’intégration contient le message d’erreur attendu du premier GET 404, et le harness l’imprime comme `FATAL ERROR` au teardown malgré un résultat final `PASS` et un code retour 0. Le 404 puis le 200 sont donc corroborés par les assertions du test et les routes observées, pas par l’absence de texte d’erreur dans le log.

## Reproduction

Forgejo scratch à la révision officielle indiquée ci-dessus; Go `1.27.3 linux/arm64`, Git `2.43.0`; tests sous `-tags sqlite`, sans service externe.

```sh
GOTOOLCHAIN=local GOPROXY=off /tmp/forgejo-v17-toolchain/go/bin/go test -count=1 -v -timeout=20m -tags sqlite ./services/pull -run 'TestPOC|TestRecoverLFSPendingMetaObjects'
GOTOOLCHAIN=local GOPROXY=off /tmp/forgejo-v17-toolchain/go/bin/go test -count=1 -v -timeout=20m -tags sqlite ./tests/integration -run '^TestPOCGenericFileLFSRecoveryUsesTargetReflogAfterRefMovesBack$'
```

Les sorties brutes, codes de retour et empreintes sont dans ce dossier. La première version rouge est conservée à part de la version verte. Les hashes identifient les sources scratch au moment de l’exécution; le hash du fichier d’intégration inclut également le test POC-065 voisin conservé dans le même fichier.

## Portée, risque et décision

Le résultat étaye une politique possible « publication acceptée au moins une fois ». Il ne rend pas Git et la base SQL atomiques : POC-060 a déjà reproduit une promotion après une lecture de ref devenue périmée. Il ne distingue pas une publication utilisateur autorisée d’une écriture administrative ou d’un rollback ultérieur, ne prouve pas l’identité d’une PR ou d’un merge, et ne transforme pas la reflog en preuve signée. La lecture complète `git reflog show` est aussi non bornée par ce prototype.

Le mécanisme dépend de la rétention Git. Forgejo v17 fixe `core.logAllRefUpdates=true` et `gc.reflogExpire=90`, mais ne fixe pas `gc.reflogExpireUnreachable`. Git documente 30 jours par défaut pour les entrées reflog dont les commits sont inatteignables (`gc.reflogExpireUnreachable`); une configuration opérateur ou locale peut changer cela. Cette échéance n’a pas été accélérée dans le test. Après perte du témoin, le récupérateur doit rester inconclusif/pending; il ne doit pas déduire un succès de l’absence de preuve. Source primaire : [Git 2.43 — git-config](https://git-scm.com/docs/git-config/2.43.0.html).

Le test n’intègre aucun worker ni appelant de production du récupérateur, ne couvre que SQLite sur un hôte, n’exerce pas PostgreSQL/MySQL, multi-nœud, stockage objet distant, expiration de reflog, SIGKILL à chaque frontière, tous les writers, ni stratégie de rétention/nettoyage. L’effort complet de maintenance upstream n’est pas chiffré. Le patch reste un POC privé non adopté; ce résultat ne justifie pas un fork. Aucune dépense, publication, modification de production, extension d’accès ou transmission de données privées n’a eu lieu.

## Fichiers et manifestes

- `source-baseline.txt`, `source-sha256.txt`, `source-hash-check.txt` : révision source et empreintes vérifiées des fichiers amont/scratch.
- `green-unit.log` / `green-unit.exit` : sept tests de récupération, code retour 0, dont le refus après expiration de la reflog.
- `green-e2e.log` / `green-e2e.exit` : scénario API fichier + récupération LFS, code retour 0.
- `red-unit.log` / `red-unit.exit` : défaut initial avant le fallback, code retour 1 attendu.
- `check_whitespace.py`, ses sorties, `gofmt-diff.*` et `diff-check.*` : vérification de format et d’espaces.
- `SHA256SUMS` : manifeste local de 21 artefacts; `sha256sum -c` passe 21/21 (manifeste auto-exclu).

La mission générale demeure incomplète : aucune comparaison contrôlée Forgejo+agent+CI n’est démontrée, le parcours LLM n’est pas établi sous un budget fiable, et les critères cumulés de refus/autorité et de maintenance de ce POC restent ouverts.
