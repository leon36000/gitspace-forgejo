# CONTINUATION-344 — portée GitLab, incident de secret et accès réseau

Date : 28 octobre 2026 (America/Montreal). Dépôt inspecté : `/home/pc6/Documents/Codex/2026-10-21-mission-construire-une-forge-con-ue/.runtime/webhook-005-workspace`, HEAD `f3e2bb3a9f9b0cb3dbcd1e03f658f0b6a05119a0`. Cible demandée : `https://pc4.tail0c55ff.ts.net/root/gitspace-forgejo.git`.

## Portée des courriels

À la demande de l’utilisateur, recherche Gmail limitée aux messages contenant le chemin GitLab, le nom `gitspace-forgejo`, PC4 et GitLab. La recherche exacte du chemin du dépôt ne retourne aucun message. Les messages d’accès PC4 retrouvés le 27 octobre comprennent :

- « Jeton GitLab pour l’agent Nexora WorkStudio et Web » : jeton destiné aux projets Nexora WorkStudio et Nexora Web avec rôle Développeur; aucune portée sur `root/gitspace-forgejo` n’est indiquée.
- « Accès GitLab et SonarQube sur PC4 » et « Mot de passe initial GitLab de PC4 » : compte GitLab `root` et mot de passe initial administrateur.

Le jeton Nexora n’a pas été utilisé pour GitLab. Sa valeur a été accidentellement incluse une fois dans une sortie d’outil; l’artefact ne la reproduit pas. Elle n’a pas été ajoutée aux commandes, fichiers, manifestes ou rapports, et aucun login GitLab n’a eu lieu. L’utilisateur a été avisé immédiatement de révoquer et renouveler ce jeton. Aucune tentative de révocation n’a été faite. Le credential administrateur initial n’a pas été utilisé, lu dans un autre message, modifié ou renouvelé; il ne sera pas utilisé automatiquement.

## Connectivité PC4

Le statut Tailscale révèle seulement le pair `pc4`, en ligne, avec l’adresse privée `100.76.101.48`. La résolution locale du FQDN retourne vide. Test TCP avec délai de 1,5 s vers cette adresse :

```text
tcp_22=unreachable (TimeoutError)
tcp_80=unreachable (TimeoutError)
tcp_443=unreachable (TimeoutError)
```

`GIT_TERMINAL_PROMPT=0 git ls-remote https://pc4.tail0c55ff.ts.net/root/gitspace-forgejo.git HEAD` échoue avant toute authentification par `Could not resolve host`. Aucun credential n’a été envoyé; aucune ref distante n’a été lue et aucune branche, commit ou push n’a été effectué. `origin` du dépôt local reste Forgejo loopback.

## Suite et limites

L’utilisateur a explicitement autorisé le transfert vers une branche GitLab dédiée, sans fusion ni écriture sur `main`. Le transfert attend que GitLab réponde depuis PC6 et que l’utilisateur configure dans le gestionnaire d’identifiants local un credential à moindre privilège, limité au projet `root/gitspace-forgejo` avec rôle Développeur et droit `write_repository`. Aucun secret ne doit être copié dans la conversation ou le dépôt. Après rétablissement, lire les refs distantes d’abord, préparer un snapshot dans un clone/worktree isolé, relire le diff, puis pousser uniquement une branche dédiée sans force-push.

L’automatisation `continuer-la-forge-locale` reste active toutes les 30 minutes avec notification seulement en cas d’échec. Son prompt a été actualisé pour inclure la migration demandée, conserver les refus de secrets et poursuivre un lot local distinct si GitLab reste bloqué. Pas de nouveau code produit ou test dans ce lot, aucune authentification ou mutation distante; résultat limité au diagnostic de reachability et de portée des accès trouvés.
