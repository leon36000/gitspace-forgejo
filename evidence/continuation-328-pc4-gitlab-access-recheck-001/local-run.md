# CONTINUATION-328 — accès PC4 GitLab réévalué après réception du courriel d’accès

Date : 2026-10-27 23:37 America/Montreal (2026-10-28 03:37 UTC). Worktree : `.runtime/webhook-005-workspace`, HEAD `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`; branche `local/WEBHOOK-005-canonical-links`.

## Courriel et gestion du secret

À la suite de l’indication de l’utilisateur, recherche Gmail exacte de `"pc4.tail0c55ff.ts.net"`, puis lecture du seul message correspondant. Le courriel fournit un accès initial avec privilèges administratifs et avertit que le mot de passe doit être changé à la première connexion. La valeur du secret, l’identifiant Gmail et les en-têtes privés ne sont pas consignés dans ce reçu ni dans le dépôt. Aucune authentification, rotation, création de jeton, requête HTTP authentifiée ou transmission au terminal n’a été effectuée.

## Connectivité vérifiée

- `tailscale ping --c 1 --timeout 3s pc4` réussit par un chemin Tailscale direct; le peer est joignable au niveau WireGuard/Tailscale.
- `getent ahostsv4 pc4.tail0c55ff.ts.net` termine avec code 2 et sans adresse; MagicDNS ne résout pas le nom fourni.
- Des HEAD HTTPS/HTTP utilisant `curl --resolve` avec l’adresse Tailscale rapportée pour le peer expirent après 6 secondes sur TCP 443 et 80. Le port SSH 22 expire après 5 secondes.
- `GIT_TERMINAL_PROMPT=0 timeout 12s git ls-remote https://pc4.tail0c55ff.ts.net/root/gitspace-forgejo.git` échoue en résolution DNS. Aucun identifiant n’a été transmis; aucune ref Git distante n’a été lue.
- Le remote `origin` local demeure `http://127.0.0.1:3130/forge-admin/agent-forge-core.git`. Aucun remote, commit, branche distante ou contenu local n’a été changé.

La couche Tailscale fonctionne, mais le service GitLab n’est pas joignable sur les ports essayés depuis ce poste. La configuration réseau/Tailscale/écoute doit être corrigée par l’opérateur PC4; aucune ACL ou règle de pare-feu n’a été modifiée.

## Admission de fusion, contrôle ciblé local

Sur la source produit existante, sans modifier code ou tests :

- `cargo test --offline --locked --test forgejo_merge_writer_admission` : 2 réussis, 0 échec. Le profil Forgejo v17 multi-writer est refusé faute de précondition CAS serveur; l’attestation single-writer est une condition explicite du profil local.
- `cargo test --offline --locked --bin forgejo-reconcile-local merge_cli_refuses_single_writer_profile_without_supervisor_capability_before_credential` : 1 réussi, 162 filtrés.
- `cargo build --offline --locked --bin forgejo-reconcile-local` : code 0. L’exécutable normal a ensuite reçu `--merge-local-test` avec le profil par défaut, fichiers d’entrée fictifs et FD `999999` inexistant. Il sort avec code 1 et JSON `verdict=inconclusive`, `server_expected_base_precondition_unavailable`, `merge_attempted=false`, sans erreur de FD ni de lecture de fichier. La source place ce refus multi-writer avant l’acquisition du credential et tout POST (branche de `run_local_test_merge_authority`, autour de la ligne 5623 dans le snapshot étudié).

Le code sépare le refus d’admission et l’exécution de fusion. CONT-327 a déjà prouvé qu’une route API de renommage Forgejo passe sous `BaseBranchMergeLock`. Les tests et le binaire vérifient le contrat fail-closed; ils ne prouvent pas qu’un superviseur réel applique l’exclusion de tous les autres writers. `VerifiedSupervisorMergeCapabilityV1` atteste le canal/process bindings, mais son résumé marque encore `supervisor_policy_independently_verified=false`. Pas de nouvelle preuve live Forgejo, de fusion ou de sécurité multi-writer dans ce lot.

## Couverture et prochaine action

Le statut Tailscale est observable au niveau peer, pas celui de l’authentification GitLab. HTTP/HTTPS/SSH ont été essayés seulement sur les ports notés; aucun login ni changement de mot de passe n’a été réalisé. Pour reprendre le transfert, rendre GitLab accessible via le nom Tailscale et un port autorisé, puis lire les refs sans force-push et préparer une branche dédiée. Au premier accès au compte initial, la rotation de mot de passe annoncée par le courriel reste une action requise, non exécutée.

Le parcours complet local MISSION-012 reste `n=1` avec adaptateur de test et writers externes non exclus. Aucun gain comparatif général n’est démontré. GitLab, coût LLM, droits GitSpace et autres critères de mission restent ouverts.
