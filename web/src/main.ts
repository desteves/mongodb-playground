import { mount } from 'svelte';
import './app.css';
import App from './App.svelte';

const target = document.getElementById('app')!;
target.innerHTML = ''; // drop the crawler-friendly fallback content
mount(App, { target });
