/*
 * EditBox.java
 *
 * Created on 11 October 2004, 12:54
 */

package net.pengo.hexdraw.original.renderer;

import java.awt.Component;

/**
 *
 * @author  Que
 */
public interface EditBox {
    
    public Component getComponent();
    
    public void setCursor(long offset);    
    
    //save the contents, 
    public void save();
    
    //called when the component is no longer needed.
    public void done();
}
