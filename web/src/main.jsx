import { createRoot, hydrateRoot } from 'react-dom/client';
import App from './App.jsx';
import './styles.css';

const root = document.getElementById('root');
const initialReleases = JSON.parse(document.getElementById('release-data')?.textContent || '[]');
if (root.querySelector('main')) hydrateRoot(root, <App initialReleases={initialReleases} />);
else createRoot(root).render(<App />);
