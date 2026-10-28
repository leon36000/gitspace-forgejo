# CONTINUATION-324 — composition MISSION-012 sous scope hôte et réseau privé

Date : 2026-10-27. Worktree produit : `.runtime/webhook-005-workspace`, HEAD source `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`.

## Question et résultat

Le processus agent peut-il être exécuté dans un namespace réellement séparé sans bus de session ni accès direct aux routes Forgejo, tout en laissant le composeur de confiance et le gate utiliser Forgejo dans une namespace privée commune ? Le parcours local complet MISSION-012 a réussi sous une fixture déterministe `local-test-adapter`, après correction du validateur qui rejetait un reçu pourtant plus strict.

Le scope systemd est créé et attesté par le lanceur sur l’hôte avant l’entrée dans Bubblewrap. Le composeur conserve l’UID effectif 1000, rejoint une namespace réseau privée, garde le scope attesté et démarre Forgejo ainsi que les étapes de confiance dans cette namespace. L’agent s’exécute dans un Bubblewrap imbriqué. Le rapport du processus agent observé côté parent confirme des namespaces PID/réseau/montage/utilisateur distincts, seulement les descripteurs 0–2, `credential_visible=false`, `host_systemd_bus_visible=false` et aucune connexion inattendue au listener MCP du gate. Depuis ce processus, les sondes API PR, Web PR et `git-receive-pack` échouent toutes pendant `connect` avec `ECONNREFUSED`; les trois chemins testés ne parviennent pas à HTTP. Le canal étroit MCP/stdio fonctionne : les vérifications prévues par le gate terminent pendant que le superviseur reste vivant.

La séquence produit une PR locale #2 à partir de la demande et des exigences versionnées; le changement réel est dans `src/lib.rs`. L’acceptation échoue d’abord sur la valeur supérieure (`left: -2`, attendue `10`) alors que le test de valeur intérieure réussit. L’échec désigne le comportement à corriger. La candidate passe ensuite les deux tests d’acceptation; le vérificateur indépendant et la revérification pré-fusion réussissent. Les tentatives de fusion avant revue et par le reviewer seul sont refusées (HTTP 405). L’autorité de fusion Rust fusionne finalement le commit candidat dans la Forgejo locale; le test d’acceptation est rejoué sur le commit de fusion exact et passe (`2 passed; 0 failed`). Le readback Forgejo montre la PR fermée/fusionnée et le commit de base égal au commit de fusion.

## Reprises et correction du harnais

- CONTINUATION-219 : `unshare --user --map-root-user --net` a lancé Forgejo sous UID effectif 0; Forgejo a refusé de démarrer en root. Le scope externe a été nettoyé (`clean`). Aucun résultat de composition n’en est déduit.
- CONTINUATION-220 : le wrapper Bubblewrap extérieur a conservé l’UID 1000 et l’admission puis les refus réseau de l’agent ont réussi. Le runner a ensuite rejeté son observation car son schéma attendu omettait `host_systemd_bus_visible=false`, champ déjà émis et exigé par l’adaptateur.
- CONTINUATION-221 : le validateur du runner exige maintenant explicitement `host_systemd_bus_visible=false`; un test de contrat refuse le champ absent ou vrai. L’exécution complète passe ensuite. Les 36 tests ciblés des deux modules concernés passent; `py_compile` et `git diff --check` passent aussi.

## Commandes, versions et provenance

L’exécution composée a été lancée depuis la racine du worktree avec :

```text
python3 evidence/continuation-221-rootless-bwrap-composition-retry-001/launch_composition.py --state-dir /tmp/mission012-attested-bwrap-221 --forgejo-binary /tmp/forgejo-17.0.0 --evidence-dir "$PWD/evidence/mission-012-live-agent-webhook-attested-bwrap-221" --execution-id live-mcp-221 --allocation-id mission012-attested-bwrap-221 --budget-cap-usd 0.20
```

Le résumé de sortie est conservé dans `launcher-summary-live-mcp-221.json`; la sortie complète non filtrée n’est pas archivée dans le dépôt. Elle comptait 30 029 octets, empreinte SHA-256 `d77f1b5911cc39e08653a70a14c1d7872e88c80f37bf584ab0c5a759e3c8627b`; stderr était vide. Le scope s’est terminé avec `scope_cleanup=clean`. Versions locales : Python 3.12.3, Bubblewrap 0.9.0, systemd 255, Linux aarch64, Go 1.27.3. Le code source Forgejo v17.0.0 audité est au commit `0d791421f85ca13a63b665053d81ea765fbbc571`; le binaire local a SHA-256 `e5ec437cd6357e8d1b4580cb47ab1daa58a32b33971a63db63446a535d33adb7`, mais le commit exact utilisé pour sa construction n’est pas établi.

Les reçus détaillés du parcours se trouvent dans `../mission-012-live-agent-webhook-attested-bwrap-221/`; ceux d’admission et de refus sont dans `../mission-012-live-agent-webhook-attested-bwrap-221-admission/`. Les manifestes SHA-256 de ces deux répertoires ont été vérifiés.

## Portée et limites

Un seul parcours local déterministe (`n=1`) est observé. Aucun appel LLM, Claude, AgentOS, RAG ou mémoire n’a été fait; le coût modèle est donc nul. Cela valide l’intégration des interfaces et la séparation de processus de cette fixture, pas un agent IA réel, les autres routes Forgejo, l’isolation contre un noyau compromis, une évasion privilégiée, les autres UID, la résistance universelle aux fuites de credential, ni un gain de productivité ou de coût. Le code et le binaire Forgejo restent locaux; aucun code amont n’est copié/adopté et aucune instance de production n’a été touchée.
