# FORGEJO-AGENT-EGRESS-LIVE-001 — probes de l’adaptateur vers une instance Forgejo réelle

Date : 25 octobre 2026. Worktree examiné : HEAD `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`, branche `local/WEBHOOK-005-canonical-links`, conservé sans reset ni commit.

## Question et parcours

Le lot MISSION-012-AGENT-EGRESS-009 testait le réseau contre un listener synthétique. Ce lot démarre une instance Forgejo v17.0.0 jetable sur `127.0.0.1:3130`, crée un dépôt public local `forge-admin/mission-012-clamp`, vérifie les routes depuis l’hôte, puis lance dans Bubblewrap la fonction `probe_sandbox_capabilities` déjà utilisée par `local-test-adapter`. Le processus sandbox tente les mêmes routes sur le vrai port Forgejo. Il ne reçoit aucun MCP config, token ni descripteur hérité; le seul fichier credential visible par son chemin est un marqueur synthétique volontairement non monté.

Commande reproductible :

```text
python3 scripts/run_forgejo_agent_egress_live_e2e.py --forgejo-binary /tmp/forgejo-17.0.0 --state-dir /tmp/forgejo-agent-egress-live-001 --evidence-dir "$PWD/evidence/forgejo-agent-egress-live-001"
```

Le binaire local annonce Forgejo `17.0.0+gitea-1.22.0` (`go1.26.9`), SHA-256 `e5ec437cd6357e8d1b4580cb47ab1daa58a32b33971a63db63446a535d33adb7`.

## Résultat observé

Contrôles positifs sur Forgejo depuis l’hôte : `/api/healthz` répond 200, la lecture API du dépôt répond 200 et la page Web du dépôt répond 200. Les routes exactes de la sonde répondent aussi depuis l’hôte : API PR 404 (la PR #1 n’existe pas dans ce fixture), page Web PR 404, et `POST .../git-receive-pack` sans authentification 401. Ces 404/401 sont des réponses de l’instance vivante, pas des succès fonctionnels de PR ou de push.

La sonde sandbox tente ces trois méthodes/chemins vers le port Forgejo 3130; chaque connexion échoue `ConnectionRefusedError`, errno 111, avant de recevoir un statut HTTP. L’inode du namespace réseau sandbox diffère de celui de l’hôte. Les contrôles hôte ci-dessus, exécutés avant la sonde, démontrent que ce refus ne venait pas d’un serveur arrêté. La sonde ne voit pas le marqueur synthétique et indique `network_connected=false`.

En parallèle, le processus de confiance reçoit un token local limité à `read:user,read:repository` et lit la branche `main` par l’API; HTTP 200. La tête de branche reste identique avant et après la sonde. Le token n’est ni transmis au sandbox, ni sérialisé dans les artefacts. L’instance Forgejo se termine avec le code 0 et le readback confirme l’absence du listener.

## Vérifications

- Le nouveau validateur de résultat passe 7/7 tests, dont refus d’un faux succès de sonde, d’un port autre que le vrai listener, d’une credential visible, d’un namespace partagé, d’un contrôle hôte défaillant, d’une lecture de confiance absente, d’un changement de branche ou d’un cleanup non confirmé.
- Suite Python complète : 167 tests réussis, 0 échec (`python-tests-all.log`).
- `py_compile` passe pour le runner, la sonde et les tests.
- Le garde Cargo confirme `license=null`, `license_file=null`, `publish=[]`, décision `no_license_selected`, droits `not_assessed`; aucune licence produit n’est choisie.
- Le test rouge du validateur (`red-live-validator.log`) échouait avant l’ajout du contrôle du port cible; la suite passe après correction.
- Le manifeste local vérifie 12/12 artefacts; les empreintes courantes du rapport, du runner et du manifeste sont ajoutées au journal global append-only.

## Portée et limites

Cette tranche exerce uniquement la fonction de sonde réseau du `local-test-adapter` dans le namespace Bubblewrap, pas son dialogue MCP complet ni du code candidat hostile. Le test prouve ici que ce namespace ne peut pas joindre les routes Forgejo locales testées, tandis qu’un lecteur de confiance séparé peut faire une lecture API. Il ne prouve pas encore qu’une vraie autorité de fusion peut merger une PR pendant que l’agent est isolé; le contrôle hôte est strictement en lecture seule. Forgejo et le sandbox partagent l’UID hôte : ce n’est pas une frontière multi-tenant. Un `receive-pack` authentifié n’a pas été tenté et aucun commit/push n’a été produit par cette sonde. Aucun LLM, AgentOS, RAG, service mémoire, coût modèle, benchmark comparatif, accès externe ou changement de production n’est impliqué.

Le répertoire privé d’instance reste sous `/tmp/forgejo-agent-egress-live-001`; le secret de l’instance et le token généré n’ont pas été exportés. Le snapshot `sandbox-probe.py`, les JSON, les journaux de test et le manifeste local sont dans ce dossier.
