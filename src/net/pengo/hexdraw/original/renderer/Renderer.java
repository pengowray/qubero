package net.pengo.hexdraw.original.renderer;

import java.awt.Graphics;

public interface Renderer {

    public int renderBytes( Graphics g, int hexStart[], long lineNumber, byte ba[], boolean selecta[], int length );
    
    public boolean isEnabled();
    public void setEnabled(boolean render);
        
    public String toString();

}


/*
 * todo two things
 * 
 * 1) allow proper canvase drawing size, either by
 *   a) g.setClipBounds like I am doing, demonstrate shinking, it shrinks drawing range.
 *   b) does scrollbar size/horizontal position manage the size of the panel viewort ?
 *      where is create(x,y) called from ?
 * 2) repaint is being called, but its not always accurate, resize fixes, or hard refresh.
 * 
 */