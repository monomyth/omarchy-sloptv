const $=id=>document.getElementById(id), v=$('v');
let Q=[],order=[],pos=0,shuffle=false,started=false,idleT;
function show(m){$('msg').textContent=m;$('msg').style.opacity=1;setTimeout(()=>$('msg').style.opacity=0,700)}
function mkOrder(){order=Q.map((_,i)=>i);if(shuffle)for(let i=order.length-1;i>0;i--){const j=Math.floor(Math.random()*(i+1));[order[i],order[j]]=[order[j],order[i]]}}
function safeUrl(u){return /^https:\/\/(x|twitter)\.com\//.test(u||'')?u:'https://x.com/'}
function cur(){return Q[order[pos]]}
function load(){const c=cur();if(!c)return;
  v.src=c.src; if(c.poster)v.poster=c.poster;
  $('aT').textContent=c.name||c.title;$('aH').textContent=c.name?c.handle:'';
  if(!c.name)fetch(c.meta).then(r=>r.json()).then(m=>{if(m.name&&cur()===c){c.name=m.name;$('aT').textContent=m.name;$('aH').textContent=c.handle}}).catch(()=>{});$('aC').textContent=c.caption||'';$('aL').href=safeUrl(c.url);
  if('mediaSession' in navigator){navigator.mediaSession.metadata=new MediaMetadata({title:c.caption||c.title,artist:c.handle,artwork:c.poster?[{src:c.poster}]:[]})}
  v.play().catch(e=>console.log('play',e));wake()}
function go(d){if(!Q.length)return;pos=(pos+d+order.length)%order.length;load()}
function togglePause(){if(v.paused){v.play();show('▶')}else{v.pause();show('⏸')}}
function fs(){const s=$('stage');
  if(document.fullscreenElement||document.webkitFullscreenElement){(document.exitFullscreen||document.webkitExitFullscreen).call(document);return}
  if(s.requestFullscreen)s.requestFullscreen().catch(()=>v.webkitEnterFullscreen&&v.webkitEnterFullscreen());
  else if(s.webkitRequestFullscreen)s.webkitRequestFullscreen();
  else if(v.webkitEnterFullscreen)v.webkitEnterFullscreen()}
function wake(){document.body.classList.remove('idle');clearTimeout(idleT);idleT=setTimeout(()=>{if(!v.paused)document.body.classList.add('idle')},3500)}
v.addEventListener('ended',()=>go(1));
let errs=0;v.addEventListener('error',()=>{if(++errs<5){show('skip');setTimeout(()=>go(1),500)}});
v.addEventListener('playing',()=>errs=0);
$('go').onclick=()=>{started=true;$('start').remove();v.muted=false;load()};
$('bPrev').onclick=()=>go(-1);$('bNext').onclick=()=>go(1);$('bPause').onclick=togglePause;$('bFs').onclick=fs;
$('bShuf').onclick=()=>{shuffle=!shuffle;const id=cur()&&cur().id;mkOrder();pos=Math.max(0,order.findIndex(i=>Q[i].id===id));show(shuffle?'shuffle on':'shuffle off')};
document.querySelectorAll('#zones div').forEach(z=>z.onclick=()=>{wake();({prev:()=>go(-1),next:()=>go(1),pause:togglePause})[z.dataset.z]()});
let sx=0,sy=0;$('zones').addEventListener('touchstart',e=>{sx=e.touches[0].clientX;sy=e.touches[0].clientY},{passive:true});
$('zones').addEventListener('touchend',e=>{const dx=e.changedTouches[0].clientX-sx,dy=e.changedTouches[0].clientY-sy;
  if(Math.abs(dx)>60&&Math.abs(dx)>Math.abs(dy)){e.preventDefault();go(dx<0?1:-1)}else if(dy<-80&&Math.abs(dy)>Math.abs(dx)){e.preventDefault();go(1)}});
document.addEventListener('mousemove',wake);document.addEventListener('touchstart',wake,{passive:true});
document.addEventListener('keydown',e=>{const k=e.key;if(k==='l'||k==='ArrowRight')go(1);else if(k==='h'||k==='ArrowLeft')go(-1);else if(k===' '||k==='p'){e.preventDefault();togglePause()}else if(k==='f'||k==='w')fs();else if(k==='o'&&cur())open(safeUrl(cur().url),'_blank','noopener')});
if('mediaSession' in navigator){navigator.mediaSession.setActionHandler('nexttrack',()=>go(1));navigator.mediaSession.setActionHandler('previoustrack',()=>go(-1))}
fetch('/queue.json').then(r=>r.json()).then(j=>{Q=j.videos;mkOrder();$('st').textContent=Q.length+' videos ready'}).catch(e=>$('st').textContent='Queue failed: '+e);
setInterval(()=>fetch('/queue.json').then(r=>r.json()).then(j=>{if(!j.videos.length)return;const id=cur()&&cur().id;Q=j.videos;mkOrder();const i=order.findIndex(k=>Q[k].id===id);pos=i<0?0:i}).catch(()=>{}),30*60*1000);
