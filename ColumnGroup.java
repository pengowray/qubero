/**
 * ColumnGroupFactory.java
 *
 * @author Created by Omnicore CodeGuide
 */

import java.util.*;
import javax.swing.table.*;

public interface ColumnGroup
{
    TableColumn getColumnClass(int col);
    
    public int setColumnCount(int count); // returns actual number of columns needed
    public int getColumnCount();
    
    public int getPreferredWidth();
    public int getMinimumWidth();
    public int getWidth();
    public void setWidth(int width);
    public void setColumnMargin(int margin); // so it knows
    
    // is it this column group that's been selected?
    public boolean getIsPrimarySelection();
    public void setIsPrimarySelection(boolean primary);
    public List getColumns();
    
    //xxx: listeners? (eg. for size changes, need redraw)
    
}

