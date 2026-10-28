# FORGEJO-AGENT-EGRESS-LIVE-001 — sondes sandbox contre Forgejo local réel

Date : 25 octobre 2026. Worktree `/home/pc6/Documents/Codex/2026-10-21-mission-construire-une-forge-con-ue/.runtime/webhook-005-workspace`, HEAD initial `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`; le worktree était modifié. Tous les tests et services sont locaux; aucune donnée privée n’a été envoyée à un service externe.

## Correction du runner

La première tentative live s’est arrêtée avant le démarrage de Forgejo. Le garde du runner autorisait `red-live-validator.log`, tandis que le dossier de preuve contenait déjà le test rouge conservé `red-route-probe.log`. Il s’agissait d’un nom d’artefact incohérent, pas d’un échec de l’isolation. Un garde explicite accepte maintenant un dossier vide ou ce seul journal rouge et refuse les artefacts inattendus. Deux tests ont d’abord échoué parce que cette fonction manquait; après ajout, la suite `tests.test_forgejo_agent_egress_live_e2e` passe 9/9. Le journal rouge antérieur reste intact.

## Parcours exécuté

Commande :

```sh
python3 scripts/run_forgejo_agent_egress_live_e2e.py \
  --forgejo-binary /tmp/forgejo-17.0.0 \
  --state-dir /tmp/forgejo-agent-egress-gate-001 \
  --evidence-dir evidence/forgejo-agent-egress-gate-001
```

Le runner démarre Forgejo `17.0.0+gitea-1.22.0` sur `127.0.0.1:3130`, crée un dépôt de test jetable puis confirme côté hôte que `/api/healthz`, la route API du dépôt et sa page Web répondent tous HTTP 200. Les contrôles hôte des routes PR API/Web sans authentification répondent 404; `git-receive-pack` répond 401. Ces réponses positives/négatives établissent que la fixture hôte cible bien Forgejo.

Sous Bubblewrap `--unshare-net --unshare-pid`, les sondes API, Web et `git-receive-pack` visent chacune le vrai port Forgejo 3130 et n’établissent aucune connexion (errno 111). L’inode du namespace réseau sandbox diffère de celui de l’hôte. Le canari synthétique n’est pas lisible depuis le sandbox. Un lecteur distinct avec scopes `read:user,read:repository` lit la branche depuis l’hôte (HTTP 200); son credential n’est pas transmis au sandbox. Le SHA de branche avant/après est identique. Forgejo termine code 0 et le listener est absent.

Sortie directe : verdict `passed`, trois routes bloquées. Le résultat complet, la provenance, le snapshot du probe et le journal rouge conservé figurent dans ce dossier. Le binaire Forgejo utilisé a SHA-256 `e5ec437cd6357e8d1b4580cb47ab1daa58a32b33971a63db63446a535d33adb7`; le commit ayant construit ce binaire n’est pas établi.

## Vérifications et portée

- `PYTHONDONTWRITEBYTECODE=1 python3 -m unittest tests.test_forgejo_agent_egress_live_e2e -v` : 9 réussis.
- `python3 scripts/test_cargo_license_state.py` : 7 réussis; `python3 scripts/verify_cargo_license_state.py` : `verified`, `license=null`, `license_file=null`, `publish=[]`, `no_license_selected`, droits `not_assessed`.
- `python3 -m py_compile` sur le runner live, l’adaptateur de test et son test; `git diff --check` : code 0.
- Python 3.12.3, Bubblewrap 0.9.0; provenance source et binaire : `provenance.json`.

Ce contrôle ferme la limite précise du rapport précédent où les sondes visaient une fixture synthétique : le listener Forgejo testé est le vrai serveur local. Le child reste `mission_012_test_agent_adapter.py`, sans LLM, AgentOS ou credential de fusion. Il s’agit d’un seul hôte et d’un contrôle réseau ciblé; cela ne démontre pas une résistance à un noyau compromis, une isolation entre UID, une défense universelle contre les sockets héritées, ni l’accès d’un agent réel. Ce contrôle n’est pas le parcours composé PR/vérification/fusion et ne mesure aucun gain comparatif. Le répertoire d’état privé sous `/tmp/forgejo-agent-egress-gate-001` est conservé; le serveur et son port sont arrêtés.
