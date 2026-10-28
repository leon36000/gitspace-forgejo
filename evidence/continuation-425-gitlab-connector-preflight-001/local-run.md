# CONTINUATION-425 — accès GitLab MCP et transfert audité du noyau de travail

Date : 28 octobre 2026 (America/Montreal). Worktree local : `local/WEBHOOK-005-canonical-links`, HEAD inchangé `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`.

## Accès GitLab et branches

L’intégration GitLab PC4 (`mcp__GitLab_PC4`, version serveur `19.5.1`, révision `449ed2875c2`) lit et écrit le projet privé `root/gitspace-forgejo`, ID `4`. La branche `codex/pc6-transfer-work-2026-10-28` existe, n’est pas la branche par défaut, n’est pas protégée et l’API rapporte `can_push=true`. Après le transfert initial, le parcours récursif paginé du dépôt contient **134 blobs**. Le SHA distant courant au contrôle est `f4573f4adade893281fc9a448146262ca48a0fd7`.

GitLab a automatiquement pris `codex/pc6-transfer-2026-10-28` comme branche par défaut et l’a protégée après le premier commit README, parce que le dépôt était vide. Cette branche contient le README bootstrap uniquement. Elle n’a pas été fusionnée ni reconfigurée; tout le contenu de travail est sur la branche non-default dédiée.

## Vérification du transfert source

L’export local inclus dans le projet contient **134 fichiers texte** (sources, tests, scripts, manifestes Cargo et documentation), totalisant environ **8,6 MB**. Les exclusions sont l’état de session `.claude`, les caches générés (`__pycache__`, `.pyc` et similaires), le checkout Git imbriqué `evidence/forgejo-v17-lfs-reflog-witness-poc-065/scratch/work/` et les chemins ignorés, dont `.runtime/`. Aucun fichier n’a été supprimé.

Le readback GitLab final a été vérifié pour les 134 blobs : les identifiants d’objets Git distants concordent avec le hash de blob recalculé depuis chaque fichier local, et les modes `100644`/`100755` concordent. Le premier essai API a stocké littéralement une chaîne Base64 au lieu de la décoder; le readback l’a détecté, et des commits suivants ont corrigé tous les fichiers concernés avec `encoding=text`. L’arbre courant est exact; l’historique contient ces commits de correction.

## Correction du readback du checkpoint

Le premier lot de reçus a remplacé le préfixe du checkpoint et les 21 fichiers de preuve ont été relus avec les blobs attendus. Le hash du checkpoint a divergé : la lecture par fenêtres a révélé que le paragraphe périmé du CONTINUATION-423 était resté dans le reste du fichier. Le commit `986b5633474988907537c638cdfacf7508bd7667` a remplacé les 100 premières lignes distantes par la fenêtre locale courante; le blob Git distant est alors devenu identique au hash local `0e0a1f06df994a87dc5994c0a5d2e4d80bb8569f`. Le reçu `transfer-manifest.sha256` est actualisé après cette correction; il sert à vérifier le lot textuel resynchronisé.

## Prévol complet des chemins locaux

Le scan frais de l’état après transfert est conservé dans `export-scan.json` : **15 212 chemins**, **15 080 fichiers réguliers**, **2 712 membres ZIP**, aucune archive illisible et **0 correspondance** aux motifs ciblés. Il couvre les chemins suivis et non ignorés du worktree, y compris les archives ZIP et leurs membres. Les formes reconnues comprennent PAT GitLab/GitHub, tokens Runner, clés OpenAI/AWS, PEM privé, identifiants dans URL et Bearer tokens. Ce résultat ne garantit pas l’absence de formats non reconnus. Les contenus `.claude`, caches, checkout Git imbriqué, index de hash et fichier de résultat sont exclus du scan de contenu sans être supprimés. Les chemins ignorés (`.runtime/` inclus), l’environnement, la mémoire des processus et le courriel ne sont pas lus.

## Limite de transfert restant

La résolution shell de `pc4.tail0c55ff.ts.net` échoue encore depuis cet environnement; un contrôle anonyme refait est archivé dans `local-route-recheck.log` (curl sort avec code 6). L’API GitLab connectée transfère le contenu texte, mais son action fichier ne décode pas les contenus binaires Base64 observés ici. Les **28 snapshots ZIP** et l’ensemble des preuves historiques ne sont donc pas transférés dans cette tranche. Le dépôt GitLab n’est pas une copie intégrale du worktree; le travail continue localement et les changements source/doc utiles sont synchronisés sur la branche dédiée. Aucun merge request, merge, job CI distant ou changement de production n’a eu lieu.

La recherche Gmail n’a pas été répétée : un reçu antérieur documente une fuite d’extraits contenant des identifiants déjà compromis. Aucun secret n’a été recopié, ni transmis à GitLab; l’accès actuel vient de l’intégration connectée.

## Suite

Synchroniser au fil des lots les documents et preuves textuelles nouvelles. Pour transférer les archives binaires et la totalité des preuves, rétablir un transport Git direct depuis un environnement qui résout et joint PC4, puis comparer les refs avant toute écriture. Garder le travail sur la branche dédiée et ne pas fusionner sans demande explicite.
