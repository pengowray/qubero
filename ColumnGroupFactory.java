/**
 * ColumnGroupFactory.java
 *
 * @author Created by Omnicore CodeGuide
 */


public interface ColumnGroupFactory
{
    Column getColumnClass(int col);
    public int getColumnCount();
    
    public int getPreferredWidth();
    public int getMinimumWidth();
    public int getWidth();
    public void setWidth(int width);
    public void setColumnMargin(int margin); // so it knows
    
    // is it this column group that's been selected?
    public boolean getIsPrimarySelection();
    public void setIsPrimarySelection(boolean primary);
    
    //xxx: listeners? (eg. for size changes, need redraw)
    
}

