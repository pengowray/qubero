/*
 * HexTable.java
 *
 * Created on 12 September 2002, 13:22
 */
package net.pengo.hexdraw.tabled;
import net.pengo.app.*;
import net.pengo.selection.*;
import net.pengo.splash.*;
import net.pengo.resource.*;
import net.pengo.data.*;

import javax.swing.table.*;
import javax.swing.*;
import java.awt.*;

/**
 *
 * @author  administrator
 */
public class HexTable extends JTable implements ResourceListener
{
    protected OpenFile openFile;
    
    private final static int hexColumns = 16; //FIXME: shouldn't be static
    
    private int[] colStart;
    private int[] colBound;
    
    protected UnifiedSelectionModel unifiedSelectionModel;
    
    /** Creates a new instance of HexTable */
    public HexTable(ActiveFile activeFile)
	{
		super();
		this.openFile = activeFile.getActive();
		
		Data data = openFile.getData();
		
		setFont(FontMetricsCache.singleton().getFont("hex"));
		TableModel tm = new HexTableModel(data,hexColumns);
		setModel(tm);
		setGridColor(Color.WHITE);
		//setRowSelectionAllowed(true); //default
		setColumnSelectionAllowed(true);
		getSelectionModel().setSelectionMode(ListSelectionModel.MULTIPLE_INTERVAL_SELECTION);
		getTableHeader().getColumnModel().getSelectionModel().setSelectionMode(ListSelectionModel.MULTIPLE_INTERVAL_SELECTION);
		//getColumnModel().setColumnMargin(0);
		calcCols();
		unifiedSelectionModel = new UnifiedSelectionModel(this);
		
		//unifiedSelectionModel.getSelectionModel().addLongListSelectionListener(openFile);
		openFile.setSelectionModel(unifiedSelectionModel.getSelectionModel());
    }
    
    /*
	 public String  getColumnName(int column) {
	 return Integer.toString(column, 16);
	 }
	 */
    
    protected void calcCols()
	{
		// FIXME: this whole thing needs a re-write..
		// FIXME: does not support resize
		// FIXME: should have n modes: 1. best-size columns (always bisect units);
		//      2. fixed sized columns, units alignment changed to group columns
		//      3. extra spacing columns, fixed size columns, fixed position units
		//      4. ascii editor simulator
		//
		int units = hexColumns;
		//int unitWidth = (int)unitPainter.getPreferredSize().getWidth(); //FIXME: precision loss. double->int
		//int spacingWidth = FontMetricsCache.singleton().getFontMetrics("hex").charWidth('W') * 5;
		int unitWidth =  FontMetricsCache.singleton().getFontMetrics("hex").charWidth('W') * 2;
		//int spacingWidth = unitWidth;
		
		float align = .5f; // .5 == put half of each space to the left side of the unit
		float alignUnit = .5f; // text will be centered on "start" point.
		
		int leftPad = 2; // how much to pad the left
		int rightPad = 2;
		
		if (units == 0)
		{
			colStart = new int[] { leftPad } ;
			colBound = new int[] { 0, leftPad + rightPad };
			return;
		}
		
		colStart = new int[units];
		colBound = new int[units + 1]; // start and end of unit, with spacing
		
		int[] spaceEvery = new int[] {1, 2, 4, 8}; //FIXME: use denominators
		//int[] spaceSize = new int[] {0, 0, 0, 0}; // none?
		int[] spaceSize = new int[] {0, 3, 10, 5};
		//int[] spaceSize = new int[] {spacingWidth, 0, spacingWidth, spacingWidth}; // original
		//int[] spaceSize = new int[] {spacingWidth/2, spacingWidth/2, spacingWidth, spacingWidth};
		
		colBound[0] = 0;
		colStart[0] = leftPad;
		
		int totalgap = leftPad + rightPad;
		
		for (int n=1; n<units; n++)
		{
			
			int gap = 0;
			for (int m=0; m<spaceEvery.length; m++)
			{
				if ((n) % spaceEvery[m] == 0)
				{
					gap += spaceSize[m];
					totalgap += spaceSize[m];
				}
			}
			
			colStart[n] = colStart[n-1] + unitWidth + gap;
			colBound[n] = colStart[n-1] + unitWidth + (int)(gap * align);
			
		}
		colBound[units] = colBound[units-1] + unitWidth + rightPad;
		colBound[units] = colBound[units-1] + unitWidth + rightPad;
		
		
		// do the setting of widths:
		float factor = ((float)getColumnModel().getTotalColumnWidth() - getColumnModel().getColumn(0).getWidth()) / totalgap;
		System.out.println("factor: " + factor);
	    
		for (int col = 0; col < hexColumns; col++)
		{
			int colWidth = colBound[col+1] - colBound[col];
			int adjustedColWidth = (int)(colWidth * factor);
			float colAlign = (float)((colStart[col]+(unitWidth*alignUnit)) - colBound[col]) / colWidth;
			//System.out.println(colBound[col] + "-" + colBound[col+1] + " / " + colStart[col] + " = " + colWidth + " " + colAlign);
			TableColumn column = getColumnModel().getColumn(col+1);
			column.setMinWidth(unitWidth + getColumnModel().getColumnMargin());
			column.setPreferredWidth(adjustedColWidth);
			//DefaultTableCellRenderer cr = new DefaultTableCellRenderer();
			TableCellRenderer cr = new HexCellRenderer(colAlign,unitWidth);
			//TableCellRenderer cr = new HexCellRenderer(0.75f,unitWidth); // test
			column.setCellRenderer(cr);
		}
    }
    
    //public ListSelectionModel getSelectionModel()
    
    public void changeSelection(int rowIndex, int columnIndex, boolean toggle, boolean extend)
	{
		//super.changeSelection(rowIndex, columnIndex, toggle, extend);
		boolean selected = isCellSelected(rowIndex, columnIndex);
		
		
		if (extend)
		{
			if (toggle)
			{
				unifiedSelectionModel.setAnchorSelectionIndex(rowIndex, columnIndex);
			}
			else
			{
				unifiedSelectionModel.setLeadSelectionIndex(rowIndex, columnIndex);
			}
		}
		else
		{
			if (toggle)
			{
				if (selected)
				{
					unifiedSelectionModel.removeSelectionInterval(rowIndex, columnIndex, rowIndex, columnIndex);
				}
				else
				{
					unifiedSelectionModel.addSelectionInterval(rowIndex, columnIndex, rowIndex, columnIndex);
				}
			}
			else
			{
				unifiedSelectionModel.setSelectionInterval(rowIndex, columnIndex, rowIndex, columnIndex);
			}
		}
		
		// Scroll after changing the selection as blit scrolling is immediate,
		// so that if we cause the repaint after the scroll we end up painting
		// everything! (from JTable)
		if (getAutoscrolls())
		{
			Rectangle cellRect = getCellRect(rowIndex, columnIndex, false);
			if (cellRect != null)
			{
				scrollRectToVisible(cellRect);
			}
		}
    }
    
    
    
    public boolean isCellSelected(int row, int column)
	{
		return unifiedSelectionModel.isCellSelected(row, column);
		/*
		 if (column == 0) {
		 return false;
		 } else {
		 return unifiedSelectionModel.isCellSelected(row, column);
		 //return super.isCellSelected(row, column);
		 }
		 */
		
    }
    
    //FIXME: make the resource listener a separate/sub class?
    public void resourceAdded(ResourceEvent e)
	{
		if (e.getSource() == this)
			return;
		
		//ResourceRemoved(this,"Selection",selectionResource);
		if (! e.getCategory().equals("Selection"))
			return;
		
		//FIXME: are you serious?
		unifiedSelectionModel.setSelectionModel(openFile.getSelectionModel());
		
		
    }
    
    public void resourceMoved(ResourceEvent e)
	{
    }
    
    public void resourcePropertyChanged(ResourceEvent e)
	{
    }
    
    public void resourceRemoved(ResourceEvent e)
	{
    }
    
    public void resourceTypeChanged(ResourceEvent e)
	{
    }

	/* (non-Javadoc)
	 * @see net.pengo.resource.ResourceListener#resourceValueChanged(net.pengo.resource.ResourceEvent)
	 */
	public void resourceValueChanged(ResourceEvent e) {
		// TODO Auto-generated method stub
		
	}
    
}
