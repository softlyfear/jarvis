import { invoke } from "@tauri-apps/api/core"
import { is_listening, isListening } from "@/stores"
import { clearInterval, clearTimeout, setInterval, setTimeout } from 'worker-timers';

// setInterval(() => {
//     (async () => {
//         is_listening.set(await invoke("is_listening"));
//     })().catch(err => {
//         console.error(err);
//     });
// }, 1000);

export function startListening() {
    (async () => {
        invoke('start_listening')
            .then((message) => {
                is_listening.set(true);
            })
            .catch((error) => {
                is_listening.set(false);
                console.error(error);
                // alert("Ошибка: " + error);
            })
    })().catch(err => {
        console.error(err);
    });
}

export function stopListening(callback: () => void) {
    (async () => {
        invoke('stop_listening')
            .then((message) => {
                is_listening.set(false);
                if(callback) {
                    callback();
                }
            })
            .catch((error) => {
                console.error(error);
            })
    })().catch(err => {
        console.error(err);
    });
}

export function capitalizeFirstLetter(string: string) {
    return string.charAt(0).toUpperCase() + string.slice(1);
}

export function showInExplorer(path: any) {
    (async () => {
        invoke('show_in_folder', {path: path})
            .then((message) => {})
            .catch((error) => {
                console.error(error);
                // alert("Ошибка: " + error);
            })
    })().catch(err => {
        console.error(err);
    });
}