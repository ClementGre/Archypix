import {StrictMode} from 'react'
import {createRoot} from 'react-dom/client'
import './index.css'
// Vidstack player skin (video + audio default layouts) — see components/photos/MediaPlayer.
import '@vidstack/react/player/styles/default/theme.css'
import '@vidstack/react/player/styles/default/layouts/video.css'
import '@vidstack/react/player/styles/default/layouts/audio.css'
import App from './App.tsx'
import {initTheme} from '@/stores/theme'

initTheme()

createRoot(document.getElementById('root')!).render(
    <StrictMode>
        <App/>
    </StrictMode>,
)
