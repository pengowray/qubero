/*
 * Created on Jan 14, 2004
 */
package net.pengo.hexdraw.original;

import net.pengo.hexdraw.original.renderer.Renderer;

/**
 * @author Smiley
 */
public class Place {

    long line;
    long addr; //
    Renderer r;
    //boolean overwrite;
    //boolean midBlink = true; // should the cursor be displayed at this point in time 
    
    public Place(long line, long addr, Renderer r){
        this.line = line;
        this.addr = addr;
        this.r = r;
    }

}
