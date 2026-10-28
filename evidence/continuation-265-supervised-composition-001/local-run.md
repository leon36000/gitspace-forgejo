# CONTINUATION-265 — composition MISSION-012 sous superviseur hôte — 27 octobre 2026

## Résultat

Le run `mission-012-live-agent-webhook-269-supervised` atteint le verdict `passed`. Dans Forgejo v17 local et privé, le parcours a admis un webhook signé, produit un changement exécutable via le MCP host, ouvert et vérifié une pull request, refusé une fusion prématurée, réévalué la preuve sur le candidat courant, puis fusionné sous gate Rust et relu le commit cible. Le test Cargo après fusion, exécuté sur le commit relu, rend `2 passed; 0 failed`. L’issue et la pull request sont fermées; le readback relie `main` au commit de fusion.

La tâche est une vraie petite correction Rust : l’implémentation clampait les valeurs supérieures à la borne minimale; l’adaptateur a observé l’échec d’acceptation (`left: -2`, `right: 10`), modifié seulement `src/lib.rs` par MCP, puis observé le succès. Le vérificateur CPU a relancé les tests indépendamment avant fusion; le post-readback a relancé les deux tests après fusion. Le reçu principal lie demande `3a3c…d431`, politique `e239…39eb`, acceptation `5d47…1dc2`, candidat `26af203bc54551f6fb74bf2e8ba88b4a92ec12fd` et merge `0b51c1e08712e5b8aac1786864925c1143fa40f4`.

## Composition et frontière observée

- Forgejo local annoncé `17.0.0+gitea-1.22.0`, binaire SHA-256 `e5ec437cd6357e8d1b4580cb47ab1daa58a32b33971a63db63446a535d33adb7`; commit de build inconnu. Bubblewrap `0.9.0`.
- Un scope systemd utilisateur temporaire, créé avant Bubblewrap, a contenu le run; son nettoyage a été confirmé `clean`. Forgejo a écouté sur `127.0.0.1:3132` dans un namespace réseau différent de l’hôte. Le probe loopback depuis l’hôte a reçu `ECONNREFUSED`; le probe de `setns` non privilégié a été refusé. Le superviseur, le gate et Forgejo partagent le namespace privé dans ce modèle.
- Le sandbox de l’adaptateur a refusé les routes API, Web et `git-receive-pack` ciblées; le credential n’était pas visible et le nombre de descripteurs ouverts était limité aux flux standards. Cela porte uniquement sur ces canaris et cette sandbox.
- Le gate a reçu le credential de fusion uniquement sur son descripteur prévu et a validé les bindings processus/canal avant et après le test. Le rapport conserve explicitement `supervisor_policy_independently_verified=false` et `external_writer_exclusion=trusted_supervisor_policy_not_independently_verified` : l’absence de writers concurrents hors du superviseur n’est donc pas prouvée.
- La barrière de vérification d’acceptation a observé le processus MCP host admis vivant, lié par PID, start-time, exécutable et namespaces pendant le contrôle sans credential. Le reçu de cycle de vie ferme explicitement ce host avant l’étape PR/merge : sa présence pendant l’autorisation de fusion n’est pas prouvée. La ligne correspondante de [la matrice des refus](../../docs/refusal-evidence-matrix.md) a été réconciliée avec cette portée exacte.
- La PR locale a d’abord reçu HTTP 405 avant approbation (`Does not have enough approvals`). La tentative du reviewer non autorisé a également reçu 405. Le webhook d’approbation n’a pas servi de preuve : les notifications PR ont été réconciliées avec l’état courant Forgejo, qui donne le commit de fusion effectif.
- La fermeture, le head `main`, le commit de merge et le résultat post-fusion sont lus dans Forgejo; le test post-fusion lie ce résultat à la base, au candidat, aux hashes d’exigences/politique/acceptation, à l’arbre de merge et au code source.

## Corrections de harnais déduites des runs conservés

Les échecs intermédiaires sont conservés et ne sont pas comptés comme réussite :

1. `265-supervised` : refus avant claim parce que le runner d’admission n’ajoutait pas la clé publique de confiance; le CLI échouait sur la politique d’attestation avant d’atteindre le test du mauvais credential.
2. `266-supervised` : après ajout de la graine et de la clé, le runner appelait un helper d’écriture absent; remplacé par le helper privé existant.
3. `267-supervised` : l’admission passait, mais le host MCP fermait la session. Un détail borné/expurgé a révélé que sa configuration webhook n’incluait pas la clé publique externe; le host charge maintenant cette clé depuis un fichier propriétaire distinct des données d’admission et l’adaptateur ne la lit pas.
4. `268-supervised` : l’adaptateur et le gate passaient, mais le receiver PR tentait de créer sous le même nom un second ancrage lié à une autre graine; le fichier propre à l’admission a reçu un nom distinct.
5. `269-supervised` : nouveau bac à sable, run complet réussi; aucune fusion n’a été tentée dans les quatre essais précédents.

Les chemins complets, reçu du superviseur et reçus d’admission sont dans `evidence/mission-012-live-agent-webhook-{265,266,267,268,269}-supervised[-admission]/`. Les ports attendus et processus suivis sont absents après les runs; les hooks Forgejo sont désactivés avant arrêt quand leur création a abouti. Les échecs n’ont pas déclenché de nettoyage ou de réutilisation d’un état antérieur.

## Changements locaux et vérification

Le harnais de mission crée une graine owner-only pour le receiver, dérive sa clé publique à partir du binaire local, la passe explicitement au CLI d’admission et la stocke dans `mission-admission-attestation.public`. Le runner MCP charge uniquement ce fichier hôte avec contrôle du propriétaire, du mode, du type, du nombre de liens et de la représentation canonique; il place la clé dans `trusted_webhook_attestation_public_keys_hex`. Le receiver de webhooks PR conserve son ancrage distinct. Le runner d’admission direct signé passe également le cinquième argument de graine au receiver.

Vérification fraîche après changements : `test_mission_012_live_mcp_agent_e2e.py` 16/16, `test_forgejo_live_webhook_mission_admission.py` 38/38, `test_mission_012_live_admission_e2e.py` 7/7 et `test_mission_012_supervised_private_netns_e2e.py` 3/3; AST Python pour les trois runners et trois modules de tests, puis `git diff --check`, passent. Logs reproductibles et hashes des six sources sont dans ce répertoire. Aucun code Forgejo ou GitSpace n’a été copié. Aucun LLM, AgentOS, RAG ou mémoire n’a été sollicité; le `local-test-adapter` déterministe rapporte zéro coût et ne démontre pas un parcours LLM.

## Réserve et suite

C’est un parcours local déterministe `n=1`, pas un benchmark ni une comparaison appariée à Forgejo + agent + CI conventionnels. Aucun gain général, baisse du coût, réduction du temps, succès LLM ou exclusion de toutes les capacités et writers n’est établi. Le superviseur est une autorité de confiance non vérifiée indépendamment; root, acteur privilégié, compromission noyau et évasion du namespace restent hors modèle. Les droits de redistribution GitSpace restent indéterminés et aucune licence produit n’est sélectionnée.

Suite locale : convertir les refus restants de la matrice (preuve absente/périmée, faux statut de l’exécutant, doublon/désordre, interruption, conflit sémantique et annulation) en scénarios composés là où le harnais les supporte; en priorité, renforcer la preuve indépendante de l’exclusion des writers et garder le credential de fusion inaccessible aux autres processus. Ensuite seulement établir un protocole expérimental apparié; aucun appel payant Claude tant qu’une limite numérique fiable et explicitement autorisée n’existe.
