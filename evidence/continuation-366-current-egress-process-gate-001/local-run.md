# CONTINUATION-366 — vérification egress sur sources courantes et état du transfert PC4

Date : 28 octobre 2026. Worktree : branche `local/WEBHOOK-005-canonical-links`, HEAD `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`. Dépôt préexistant fortement modifié; aucune remise à zéro, commit ou publication n’a été faite.

## Retest justifié par les empreintes

Le checkpoint CONT-365 redemandait une vérification processus/réseau déjà présente dans plusieurs reçus. La comparaison des hashes a montré que les sources de `FORGEJO-AGENT-STDIO-MCP-MERGE-001` du 26 octobre différaient du checkout courant, et que les fichiers runner/adaptateur de CONT-307 avaient aussi divergé sur trois fichiers. Je n’ai pas rejoué les anciennes fusions. Le parcours isolé présent a capturé les hashes des sources exécutées; les sept empreintes de `source-current-check.json` correspondent au code du worktree au moment du run.

La première invocation a échoué au préflight (`FileNotFoundError`) parce que le répertoire de preuve n’avait pas encore été créé; l’échec est conservé dans `first-attempt.json`. L’inspection du chemin d’exécution confirme que `evidence_dir.resolve(strict=True)` intervient avant la création du state dir et le lancement de Forgejo. Aucune instance ni state dir n’a été créé pour cette tentative. Après création du répertoire de preuve, le même runner réussit.

## Instance Forgejo locale et frontière agent/gate

Le runner démarre une instance jetable Forgejo `17.0.0+gitea-1.22.0`, binaire SHA-256 `e5ec437cd6357e8d1b4580cb47ab1daa58a32b33971a63db63446a535d33adb7`, Go 1.26.9. Le commit de construction exact du binaire n’est pas établi. Les contrôles hôte atteignent le healthcheck, l’API du dépôt et la page Web du dépôt en HTTP 200; l’API/Web de PR #1 renvoient 404 car aucune PR n’existe dans ce scénario, et le POST receive-pack sans authentification renvoie 401/403.

Le vrai processus fixture `mission_012_test_agent_adapter.py --stdio-mcp` tourne sous Bubblewrap dans des namespaces réseau et PID distincts. Ses connexions directes vers les routes API PR, Web PR et `git-receive-pack` échouent toutes à `connect` avec errno 111; `network_connected=false`, les seules FD visibles sont `[0,1,2]`, le canari de credential n’est pas visible et le superviseur observe zéro connexion de route. Le parent vérifie indépendamment que le processus descendant exact est vivant à la barrière MCP.

Pendant que l’enfant reste à la barrière, le gate de confiance lit la branche `main` par GET API (HTTP 200); le SHA avant et pendant le gate est identique : `b672e4456ed378c957cffc543ae5405d9a84a156`. Le credential synthétique à portée d’écriture reste dans la mémoire du runner et un memfd CLOEXEC; il n’est ni persisté dans un fichier, ni sérialisé dans la commande/configuration/environnement MCP, ni hérité par l’enfant. La lecture du memfd sous `/proc` est refusée depuis son namespace PID. Cette exécution ne fait aucun POST de fusion (`merge_route_called=false`) : elle valide la séparation pendant un GET gate, pas un merge sur les sources courantes.

## Échec actionnable et correction dans la fixture

L’adaptateur local-test-adapter lit son contexte via MCP et modifie la fixture Rust par `workspace_write_text`. Le test d’acceptation échoue d’abord (`cargo test --offline --locked`, exit 101; valeur observée `-2`, attendue `10`), ce qui donne une correction précise. Après le changement MCP, le cargo test du workspace fixture passe (exit 0). Cela prouve le transport de contexte/outils et un échec suivi d’une correction sur une fixture déterministe; ce n’est pas un run LLM/AgentOS, ni une PR de changement fusionnée, ni une vérification indépendante du modèle.

## Vérifications et nettoyage

Le scénario composé retourne `verdict=passed`; Forgejo sort normalement (`returncode=0`) et le listener `127.0.0.1:3130` est absent après le run. Le test ciblé des composeurs passe **6/6** (`ComposedStdioFixtureConsistencyTests`, `ForgejoActionsConfigurationTests`, `NativeActionsDeliveryBindingTests`); `git diff --check` passe. La commande, résultats assainis, toolchain et hashes sont conservés dans ce dossier.

Le répertoire de données jetable du runner demeure dans `/tmp/forgejo-cont-366-current-egress-state-001`; il n’a pas été supprimé. Aucun service de production, code Forgejo source, GitLab, courriel ou modèle payant n’a été utilisé dans ce run.

## Transfert GitLab demandé

L’utilisateur réitère que les accès sont dans son courriel. L’examen ciblé antérieur CONT-344/336 est conservé; cette reprise n’a pas relu les messages ni utilisé d’identifiant. Le PAT précédemment trouvé vise un autre projet et a été exposé dans une sortie d’outil; il ne doit pas être réutilisé et doit être révoqué/renouvelé. Le mot de passe initial `root` n’a pas été utilisé. Une sonde anonyme actuelle, sans requête HTTP, ne résout toujours pas `pc4.tail0c55ff.ts.net`; TCP 22 et 443 expirent vers l’adresse Tailscale précédemment observée. Aucun `ls-remote`, lecture de refs ou accès GitLab n’a eu lieu. Détails : `pc4-connectivity-readonly.txt`.

L’inventaire de transfert CONT-336 est déjà réalisé. Le nouveau delta du checkout compte 19 fichiers suivis modifiés, 0 staged et 14 035 chemins non suivis/non ignorés (`git ls-files --others`), soit 177 657 013 octets sur ces chemins et fichiers modifiés. Aucun lien symbolique ni fichier >10 MiB dans cet échantillon; les motifs connus de PAT GitLab/GitHub, clés privées, clés AWS/OpenAI et affectations littérales de secret n’ont pas matché les fichiers parcourus hors du répertoire Claude local, qui reste exclu et non inspecté. 446 chemins correspondent au filtre large de nom contenant credential/token; ils incluent des rapports et tests, mais un nom seul ne prouve pas la présence d’une valeur secrète. Ce scan heuristique ne garantit pas l’absence de secrets; les caches/artefacts ajoutés depuis CONT-336 doivent être réconciliés avant le transfert.

## Limites et suite

Un parcours local `n=1` démontre l’isolement réseau de l’adaptateur dans cette configuration Bubblewrap/Linux et le GET autorisé du gate. Le parent et Forgejo partagent le même UID; aucune garantie multi-tenant, de résistance à une évasion noyau, de filtrage de toutes destinations, d’exclusion des writers externes ou d’atomicité de fusion n’en découle. `local-test-adapter` est un substitut d’interface; aucun AgentOS/LLM, CI conventionnelle comparée, mesure de tokens/coût, PR/merge actuel ou gain général n’est établi. La mission demeure incomplète.

Prochaine action locale : réconcilier l’audit d’export CONT-336 avec les nouveaux chemins/caches depuis ce reçu, puis choisir dans la matrice un refus réellement non couvert sans rejouer les parcours. Pour le transfert PC4, l’utilisateur doit révoquer le PAT déjà exposé, configurer un credential frais limité au dépôt et rétablir HTTPS 443 ou SSH 22 via Tailscale. Ensuite seulement, lire l’identité et les refs avant une branche dédiée non destructive.
