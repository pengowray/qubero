/*
 * Created on Jan 11, 2004
 */
package net.pengo.hexdraw.original.renderer;


public interface RendererListener {
    /** call when the width has changed. do NOT call rendererDisplayUpdated() as well */
    public void rendererWidthUpdated();
    
    /** call when the width has not changed but the (method of) display has */
    public void rendererDisplayUpdated();
    
    public void rendererEnabledUpdated(Renderer r);
}
