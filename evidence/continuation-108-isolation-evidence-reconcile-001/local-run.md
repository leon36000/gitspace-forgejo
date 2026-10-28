# CONTINUATION-108 — rapprochement des preuves d’isolation Forgejo déjà conservées

Date : 26 octobre 2026. Base examinée : `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`. Le worktree est fortement modifié. Cette tranche est un audit local en lecture seule des preuves existantes; aucun service, agent, route Forgejo ou fusion n’a été relancé.

## Sources vérifiées

Le script de vérification lit les trois manifestes locaux append-only et valide le dernier hash par chemin. Résultats :

- `forgejo-stdio-mcp-same-candidate-001` : 42 entrées, 42 chemins courants vérifiés; manifeste SHA-256 `40fc76abe161fbd666a69a3132cb663f010661b66763ac459150483ebc5adbd8`.
- `continuation-105-private-netns-webhook-001` : 26 entrées historiques, 23 chemins distincts, 23 dernières entrées vérifiées; manifeste SHA-256 `390e8ea5a34034afb948eb9d16108a6afc5888b4467b54847f651c2ed3692b88`. Trois entrées historiques de `local-run.md` sont remplacées par les suivantes dans le manifeste append-only; seule la plus récente est comparée au fichier courant.
- `forgejo-agent-stdio-mcp-merge-001` : 27 entrées, 27 chemins courants vérifiés; manifeste SHA-256 `8051bf0173465e33d55ae6b20e502ab9f68f4b9ca99983bc917e96d5bad07e37`.

Les hashes exacts des rapports et JSON utilisés, ainsi que les observations non sensibles extraites, figurent dans `cross-audit.json`. Les préfixes des index globaux au début de cette tranche sont conservés dans ce même fichier : worktree 10 455 lignes, SHA-256 `cc307f64aae9b2c96e61e7c4c71c90f1169810f74d72cff429ceeec29e956174`; projet 4 550 lignes, SHA-256 `0ae34b79c91b4784dabcbfc006c845a469dd8ee4cb055021ef691e817bcf26de`.

## Conclusion limitée

Les tests demandés de réseau agent et de confidentialité du credential existent déjà : dans `FORGEJO-STDIO-MCP-SAME-CANDIDATE-001`, l’agent MCP n’atteint aucune des routes API/Web/`git-receive-pack`, n’a que les FDs 0–2 et ne reçoit pas le credential; la lecture canonique du gate répond HTTP 200 et son merge local répond HTTP 200. Une preuve indépendante de merge indique aussi ces refus et une fusion gate autorisée. Ces deux rapports déclarent explicitement `external_writers_excluded=false` : ils n’autorisent pas le profil single-writer face aux writers externes.

La seconde preuve pertinente, `CONTINUATION-105`, met Forgejo réel et le réconciliateur webhook dans un namespace privé commun; la lecture autorisée du gate y fonctionne et le loopback hôte reçoit errno 111. Son enveloppe d’observation brute était faussement marquée `failed`; le JSON de réévaluation, qui vérifie 10 assertions et les hashes des données sources, donne `passed`. Ce parcours ne fusionne pas de PR.

Ces runs séparés ne peuvent pas être assemblés en attestation. Aucun des deux ne prouve dans une même exécution supervisée l’identité des processus Forgejo et autorité de fusion, la séparation du processus agent, l’exclusion des writers hôte, et une décision de fusion liée au reçu. Le refus CLI introduit en CONTINUATION-106 reste donc correct. Le checkpoint et la décision d’architecture ont été corrigés pour pointer vers la lacune restante, sans changement de règle ni de runtime.

## Prochaine action

Concevoir le contrat de l’autorisation superviseur à usage unique, puis l’éprouver contre absence, falsification, expiration, rejeu, changement de PID/namespace et mismatch de cible avant d’autoriser une fusion. Le premier parcours positif devra réunir sous une supervision unique Forgejo, le gate et l’agent séparément isolé; aucun profil ne sera activé par un booléen appelant ou par la composition de rapports historiques.

## Portée

Aucun nouveau comportement n’est implémenté ni testé; ce lot corrige une prochaine action devenue redondante et évite de refaire les parcours. Il ne constitue pas une preuve de sécurité supplémentaire ni un gain comparatif. Aucun appel LLM, coût, accès privé externe, publication ou environnement de production n’est impliqué. Le projet reste incomplet.
