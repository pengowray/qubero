/**
 * AbstractColumnGroup.java
 *
 * A bunch of columns (or one column). Usually with the same renderer.
 *
 * @author Created by Omnicore CodeGuide
 */

package net.pengo.hexdraw.tabled;

import javax.swing.table.*;
import java.awt.*;

public abstract class AbstractColumnGroup
{
    abstract public int getColumnCount();
    //abstract public Column getColumn(int col);
    
    abstract public int getPreferredWidth();
    abstract public int getMaximumWidth();
    abstract public int getMinimumWidth();
    
    // set width of entire column group
    //public void setWidth(...);
    
    // set width of specific column.. may or may not affect column group width
    //public void setColumnWidth();
	
}

