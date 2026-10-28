# Hermes et interfaces de modèle locales — contrôle ciblé

Date d’observation : 26 octobre 2026. Contrôle local en lecture seule depuis le worktree produit, à la base `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`. Le reçu JSON contient les empreintes et résultats structurés.

## Constat

Le dossier `/home/pc6/NAOS/60_travaux/GitSpace_Hermes_20261011` contient quatre profils Hermes/Kanban et un dispatcher natif. Le lanceur `/home/pc6/.local/bin/hermes` est un wrapper shell de 70 octets vers un binaire local de 7 566 octets; leurs SHA-256 sont dans le reçu. Je n’ai exécuté ni le wrapper, ni le binaire, ni le dispatcher. Les fichiers de profils ont été analysés en mémoire avec une liste de champs non sensibles; aucune valeur de clé ou fichier `.env` n’a été lu ou imprimé.

Les unités `gitspace-hermes-dispatch.timer` et `gitspace-hermes-dispatch.service` étaient inactives. Le script d’activation référence un interpréteur Python sous `GitSpace_DSH_Eval_20261009` qui n’existe pas. La configuration seule ne prouve donc pas un runtime de mission utilisable. Une route définie sur `127.0.0.1:18089` peut passer par LiteLLM vers un fournisseur distant; elle n’a pas été contactée.

`ss` a montré des listeners loopback sur les ports 18086, 18088, 18089 et 9119. Des GET sans authentification ni corps vers `/v1/models` sur 18086 et 18088 ont tous deux reçu une réinitialisation de connexion. Le port 18089 et le tableau de bord 9119 n’ont pas été sondés. `forge-gateway.service` et `hermes-dashboard.service` étaient inactifs malgré le listener 9119; son propriétaire n’a pas été inspecté. Aucun listener n’a été observé sur 8787 ou 11434. Ces observations ponctuelles ne donnent ni provenance ni version/licence du modèle.

## Effet sur l’architecture

Le seul adaptateur exécutable constaté dans le produit demeure `FixtureRuntime` sous tests. Aucun adaptateur AgentRuntime de production vers Hermes ou AgentOS n’est établi. Hermes doit rester un candidat à intégrer via l’interface versionnée existante après validation de la frontière de données, de l’authentification locale, de l’isolation et du coût des routes; son Kanban ne justifie pas un second ordonnanceur. Pour ce lot, aucun agent ni modèle n’a été utilisé et aucun coût d’inférence n’a été engagé.

## Commandes et limites

Contrôles effectués : lecture et empreinte du wrapper/binaire et des fichiers d’ops; état `systemctl is-active` de cinq unités; inventaire des seules adresses/ports loopback sélectionnés par `ss -ltnH`; parsing YAML avec sortie limitée à des champs autorisés; deux requêtes GET de métadonnées locales sans credential. Le dispatcher, les profils, les journaux, les variables d’environnement, les clés, les processus et la route potentiellement payante n’ont pas été invoqués ou inspectés.

Ceci n’est ni un test d’inférence, ni une intégration Hermes, ni une preuve qu’aucun modèle local n’existe. Le projet reste incomplet.
