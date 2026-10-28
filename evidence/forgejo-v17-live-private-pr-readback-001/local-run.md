# FORGEJO-V17-LIVE-PRIVATE-PR-READBACK-001

## Résultat

Un scénario local unique (`n=1`) a réussi contre une instance Forgejo 17.0.0 privée et isolée. Il a créé/réutilisé `forge-admin/agent-forge-core`, l’issue 1, l’exigence versionnée `requirements/REQ-IDENTITY-001.v1.json` et la PR 2. Le candidat contient d’abord un parseur qui accepte à tort les espaces et chemins supplémentaires. Le vérificateur indépendant expose trois contre-exemples; la correction est publiée sur la branche de PR et passe le même vérificateur.

Le vrai receiver et le vrai CLI Rust du worktree ont ensuite traité une livraison HMAC de test. Le CLI a lu l’objet PR privé dans Forgejo par un proxy loopback étroit, avec un jeton synthétique aux scopes read-only `read:repository,read:user` reçu par `--host-credential-fd`. Il a fait un seul GET authentifié et retourné les SHA actuels head/base. Une seconde exécution sans credential reçoit HTTP 404, produit `inconclusive/resource_not_found`, n’émet aucun état canonique et ne modifie pas l’inbox. Le jeton temporaire est supprimé, puis son absence est relue dans l’API.

Le rapport structuré sans secret est [run.json](run.json); `stderr.log` est vide. Le runner est [run_local_forgejo_v17_private_pr_readback.py](../../scripts/run_local_forgejo_v17_private_pr_readback.py).

## Environnement et provenance

- Image officielle locale `codeberg.org/forgejo/forgejo:17.0.0`, épinglée par ID `sha256:06d38310de496a10186d952bdad5ca764c153c8023ede8dfa0944c79a1dc5412`. Le binaire rapporte `forgejo version 17.0.0+gitea-1.22.0`, compilé avec Go 1.26.9. Le conteneur annonce le dépôt source Forgejo et la licence `GPL-3.0-or-later`; le commit exact qui a produit cette image n’est pas établi.
- Conteneur dédié sur le réseau Docker local `agent-forge-forgejo-v17-20261025`, ID `2485044f69b4d334d140dc0bc769697a20277988a6e6e53bf3ebfee401bfc136`, `Internal=true`, sans port hôte publié. L’API de test est joignable depuis l’hôte par son IP privée; le proxy n’écoute que sur `127.0.0.1:3130` et n’accepte que le GET de cette PR.
- Python 3.12.3, Bubblewrap 0.9.0, Cargo 1.95.0, Docker 29.6.2. Le vérificateur exécute uniquement le fichier candidat monté en lecture seule, avec `--unshare-net --unshare-pid`; ce n’est pas un sandbox multi-tenant général.
- Worktree au HEAD `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`; le worktree était fortement modifié. `cargo build --offline --bin forgejo-webhook-local --bin forgejo-reconcile-local` termine avec succès (artefacts incrémentaux déjà à jour). SHA-256: runner `425df29e6b8420033ce7722c059fd6869aad407c32d347f4b8243e42bd5de1c3`; `forgejo-webhook-local` `4b998554550104d885e9827520d4868f9d639e344a5c3888e46f9fe31ad61761`; `forgejo-reconcile-local` `8fb1b2fd1548d2d3aeab9fc4f38c9b39a8d7318cb11d10326ac9f77fe573cec3`.

Le scratch source Forgejo v17 déjà audité au commit `0d791421f85ca13a63b665053d81ea765fbbc571` est une copie fortement modifiée et n’a pas servi à construire cette image. Aucun code Forgejo ni GitSpace n’est copié dans le produit.

## Commandes et observations

1. `cargo build --offline --bin forgejo-webhook-local --bin forgejo-reconcile-local` — exit 0; sortie dans [build.log](build.log).
2. `python3 -m py_compile scripts/run_local_forgejo_v17_private_pr_readback.py` — exit 0.
3. `python3 scripts/run_local_forgejo_v17_private_pr_readback.py` — exit 0; sortie JSON dans [run.json](run.json).

Le run final a commencé avec le défaut réintroduit explicitement dans la branche isolée; le runner a confirmé l’échec initial, ajouté la correction, puis relu l’objet PR pour obtenir le SHA head **après** cette correction. Le vérificateur isolé sort les contre-exemples `alice /repo`, `alice/repo ` et `alice/repo/extra` avec exit 1, puis `PASS: 2 valid and 5 invalid repository coordinates` avec exit 0. Le readback authentifié correspond au head `1a81548a1fa6511513b6a8c61da840515928c23d` et à la base `8bc9a151bf6a1ea387bd9609edded8eee03587a8` renvoyés par l’API vivante.

Le receiver a accepté l’événement de webhook construit et signé par le banc (`HTTP 202`). Il ne s’agit pas d’un événement capturé auprès de Forgejo. L’instance est un vrai collaborateur Forgejo local, mais le scénario, son issue, ses fichiers, sa PR et son acteur sont fournis par `local-test-adapter`; aucun modèle, AgentOS, RAG ou mémoire réelle n’intervient.

## Limites

- Le run valide une capacité de lecture courante, pas une preuve de vérification ni une autorisation de fusion. Aucune fusion n’a été tentée : le scénario ne démontre pas de précondition CAS serveur face aux autres writers.
- Le code CLI `cmd/admin_user_generate_access_token.go::runGenerateAccessToken` au commit source officiel audité `0d791421f85ca13a63b665053d81ea765fbbc571` fixe `ResourceAllRepos=true`. Les scopes du jeton sont read-only, mais le jeton n’est pas restreint à un dépôt précis; il n’est utilisé que par le compte synthétique de l’instance interne de test. Le blob source de ce fichier est inchangé dans le scratch audité.
- L’issue et l’exigence sont des données de test, sans arbitrage humain. Le vérificateur est indépendant du processus exécutant le code mais reste un script du banc, pas le moteur de preuve produit.
- Le résultat est `n=1`, sans baseline appariée Forgejo + agent + CI; il ne mesure ni gain de productivité, ni coût/tokens de modèle. Aucune dépense externe et aucun service externe ne sont utilisés.
- Le parcours complet de mission, l’agent réel, la preuve de fusion non contournable et la validation après fusion restent à démontrer ailleurs; ce lot ne complète pas le projet.
