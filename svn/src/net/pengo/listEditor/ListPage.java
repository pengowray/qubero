/*

Qubero, binary editor
http://www.qubero.org
Copyright (C) 2002-2004 Peter Halasz

This program is free software; you can redistribute it and/or
modify it under the terms of the GNU General Public License
as published by the Free Software Foundation; either version 2
of the License, or (at your option) any later version.

This program is distributed in the hope that it will be useful,
but WITHOUT ANY WARRANTY; without even the implied warranty of
MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
GNU General Public License for more details.

The GNU General Public License is distributed with this application, or is
available at:
- http://www.qubero.org/license.html
- http://www.gnu.org/copyleft/gpl.html
- or by writing to Free Software Foundation, Inc., 
  59 Temple Place - Suite 330, Boston, MA  02111-1307, USA. 

*/

/**
 * listPage.java
 *
 * @author Peter Halasz
 */

package net.pengo.listEditor;
import net.pengo.propertyEditor.EditablePage;
import net.pengo.propertyEditor.PropertiesForm;
import net.pengo.resource.ListResource;
import javax.swing.JTable;
import javax.swing.JLabel;
import javax.swing.table.TableModel;
import javax.swing.table.DefaultTableModel;
import net.pengo.propertyEditor.MultiPage;
import net.pengo.resource.IntPrimativeResource;
import net.pengo.propertyEditor.SummaryPage;
import net.pengo.propertyEditor.TextOnlyPage;
import net.pengo.propertyEditor.PropertyPage;
import javax.swing.tree.DefaultMutableTreeNode;
import java.util.Iterator;
import net.pengo.resource.Resource;
import java.util.ArrayList;
import java.util.List;

public class ListPage extends MultiPage
{
	private ListResource list;
	
	public ListPage(PropertiesForm form, ListResource listResource)
	{
		super(form, "list or array");
		
		this.list = listResource;
		
			List subpageList = new ArrayList();
			
		//TextOnlyPage Page1 = new TextOnlyPage("item1", "" + list.elementAt(new IntPrimativeResource(0)));
		//TextOnlyPage Page2 = new TextOnlyPage("item2", "" + list.elementAt(new IntPrimativeResource(1)));
		
		for (Iterator i = list.iterator(); i.hasNext(); ) {
			Resource r = (Resource)i.next();

			//fixme: evaluate, really? shouldn't this be an option or smoething?
			PropertyPage pp = r.evaluate().getValuePage();
			if (pp != null) {
				subpageList.add(pp);
			}
		}
		
		setPages((PropertyPage[])subpageList.toArray(new PropertyPage[0]));
		
		/*
		JTable table = new JTable();
		DefaultTableModel dataModel = new DefaultTableModel();
		dataModel.addColumn("#",new Integer[]{new Integer(1),new Integer(2),new Integer(3)});
		dataModel.addColumn("#",new String[]{"abc","def","ghi"});
		table.setModel(dataModel);
		add(table);
		 */
		//add(new JLabel("test"));
		
		
	}
	
	protected void saveOp()
	{
		// TODO
	}
	
	public void buildOp()
	{
		// TODO
	}
	
	public String toString()
	{
		return "List/Array editor";
	}
	
	
}

