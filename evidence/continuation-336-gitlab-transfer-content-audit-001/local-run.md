# CONTINUATION-336 — audit local avant transfert GitLab

Date : 28 octobre 2026. Dépôt inspecté : worktree local `local/WEBHOOK-005-canonical-links`, base `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`. Cible demandée par l’utilisateur : `https://pc4.tail0c55ff.ts.net/root/gitspace-forgejo.git`.

## Inventaire

L’audit compare HEAD à l’arbre de travail, inclut les chemins non ignorés non suivis, déduplique les entrées et exclut son propre dossier de ce décompte. Il trouve 19 chemins suivis modifiés, 13 442 chemins non suivis et 13 461 candidats distincts avant le présent reçu. Un de ces chemins est la racine d’une worktree Git imbriquée; le comptage de fichiers ordinaires est donc inférieur d’une unité. L’arbre `evidence/` mesurait 378 084 003 octets hors dossier de ce reçu (environ 360,5 MiB); 28 snapshots ZIP totalisent 107 151 276 octets et 2 712 entrées. Aucun fichier candidat ne dépasse 20 MiB; aucun lien symbolique n’a été trouvé. Le dépôt Git local contient 1 800 objets lâches, 71,72 MiB, sans pack; ce n’est pas une mesure du futur pack réseau.

L’index racine a 22 lignes `.pyc` et 24 lignes `.zip`, représentant respectivement 21 et 23 chemins distincts. Parmi les 119 fichiers bytecode directs, 21 sont référencés par l’index; 98 sont des caches non référencés à exclure du transfert. Les fichiers d’évidence mentionnés par les index doivent rester pour rendre leurs sommes vérifiables. Les 28 ZIP sont conservés dans le périmètre demandé. `.claude/ralph-loop.local.md` est un fichier d’instruction local et est exclu. Les chemins ignorés par `.gitignore`, notamment `target/` et `.tools/`, ne sont pas candidats. Les paramètres `origin` restent ceux du Forgejo loopback local; ils ne contiennent pas d’identifiant GitLab.

Un dépôt bare de test et sa worktree propre se trouvaient sous `evidence/forgejo-v17-lfs-reflog-witness-poc-065/scratch/`. La worktree est à la même tête `d5bd7f05d5e350dcba90ef3fceb101f4fb07a21b` que le bare repo. Un bundle local de 304 octets représente l’historique complet sans transformer le dépôt imbriqué en gitlink; `git bundle verify` réussit. Les dépôts locaux originaux restent intacts. Le JSON d’inventaire comprend la somme SHA-256 du bundle, les références, les tailles, les nombres et les résultats de scan.

## Détection de secrets et limites

Le scan local vérifie les motifs usuels de PAT GitLab/GitHub, clés privées, clés OpenAI/AWS, URL avec userinfo et affectations littérales de secrets. Aucun nom de fichier `.env` ou `credentials.json` n’a été trouvé; aucune valeur littérale d’identifiant n’a été détectée dans les fichiers texte. Le scan des 28 ZIP (2 712 entrées) n’a trouvé aucun de ces motifs ni fichier d’entrée portant un nom de credential. Trois correspondances d’une heuristique large dans des fichiers Python étaient des expressions de lecture/génération, sans valeur littérale. Une détection d’URL dans un `.pyc` local non référencé provenait du pool de chaînes d’un test négatif localhost; ce cache sera exclu. Ce scan est une vérification heuristique, pas une garantie cryptographique d’absence de secrets.

Les identifiants mentionnés dans le courriel ne sont ni enregistrés dans l’audit ni envoyés à GitLab. Cette reprise n’a fait aucune authentification, aucun `ls-remote`, aucune lecture de refs, aucun commit ou push. PC4 répond au ping Tailscale, mais MagicDNS ne résout pas le FQDN et les ports TCP 22/80/443 restent inaccessibles depuis ce poste; les opérations distantes restent bloquées avant authentification.

## Transfert prévu et preuve

Une fois le service joignable, lire d’abord les refs GitLab et confirmer l’état du dépôt. Préparer une branche dédiée à partir du snapshot local complet, sans réécriture ni force-push, après exclure le fichier `.claude` local, les 98 caches non indexés et le dépôt bare représenté par son bundle. Préserver les fichiers d’évidence indexés et les manifestes append-only. Ne pas créer le commit de transfert ni pousser avant cette lecture des refs.

Reproduction : `python3 evidence/continuation-336-gitlab-transfer-content-audit-001/audit_transfer_contents.py`; bundle : `git bundle verify evidence/continuation-336-gitlab-transfer-content-audit-001/forgejo-v17-lfs-reflog-witness-server.bundle`. La sortie expurgée est `transfer-content-audit.json`; le bundle et les empreintes sont dans ce dossier. Le remote GitLab n’a pas été contacté.
