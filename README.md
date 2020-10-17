# devcontainers-rs

For those who want to try out VSCode's devcontainers without VSCode.

This project is mostly aiming to bring the VSCode's remote development experience to neovim, but at
the end others editors/clients can be attached too.

Please refer to the devcontainer.json [reference](https://code.visualstudio.com/docs/remote/devcontainerjson-reference)

**⚠️ This is in actively development and rapidly growing! Use at your own risk. And feel free to
play around and let me know in the issues what features you'd like to see here. ⚠️**

## Requirements

- docker

## HOW-TO

0- Run `devcontainers_rs -h` to see the available options.

1- Inside a directory containing the `.devcontainer` folder, run:

```bash
$ devcontainers_rs up
```

This will start the proper containers based on the settings provided in `.devcontainer/devcontainer.json`

1.1 - If u want to spawn a custom editor/ide after creating the containers, add this to your `devcontainer.json`:

```json
{
    ...
    "application": { "cmd": ["nvim-qt", "--server", "9797", "--nofork"] },
    ...
}
```

## FEATURES:

⚙️ - DOING
✅ - DONE

[⚙️] create containers based on image

[⚙️] spawn custom application

[⚙️] `postCreateCommand`, `postStartCommand`, `postAttachCommand`

[⚙️] `appPort`

[ ] `devPort`

[ ] `forwardPorts`

[ ] `initializeCommand`

[ ] create containers based on `build`

[ ] create containers from docker-compose

[ ] destroy/stop containers

[ ] user management (`remoteUser`,  `containerUser`, `updateRemoteUserUID`)

