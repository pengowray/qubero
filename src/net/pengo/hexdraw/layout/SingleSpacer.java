/*
 * SingleSpacer.java
 *
 * Created on 7 December 2004, 08:25
 */

package net.pengo.hexdraw.layout;

/**
 *
 * @author  Que
 */
public abstract class SingleSpacer extends SuperSpacer {
    
    /** Creates a new instance of SingleSpacer */
    public SingleSpacer() {
    }
    
    public boolean isMulti() {
        return false;
    }
    
    public long getSubSpacerCount() {
        return 1;
    }
}
