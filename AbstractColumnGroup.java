/**
 * AbstractColumnGroup.java
 *
 * A bunch of columns (or one column). Usually with the same renderer.
 *
 * @author Created by Omnicore CodeGuide
 */

import javax.swing.table.*;
import java.awt.*;

public abstract class AbstractColumnGroup
{
    public int getColumnCount();
    public Column getColumn(int col);
    
    public int getPreferredWidth();
    public int getMaximumWidth();
    public int getMinimumWidth();
    
    // set width of entire column group
    //public void setWidth(...);
    
    // set width of specific column.. may or may not affect column group width
    //public void setColumnWidth();
	
}

