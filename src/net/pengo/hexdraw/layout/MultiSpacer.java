/*
 * MultiSpacer.java
 *
 * Created on 7 December 2004, 08:18
 */

package net.pengo.hexdraw.layout;

import net.pengo.bitSelection.BitCursor;

/**
 *
 * @author  Que
 */
public abstract class MultiSpacer extends SuperSpacer{
    
    /** Creates a new instance of MultiSpacer */
    public MultiSpacer() {
    }
    
    public boolean isMulti() {
        return true;
    }
}
