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

/*
 * AbstractResourcePropertiesForm.java
 *
 * Created on August 23, 2003, 6:38 AM
 */

package net.pengo.propertyEditor;

/**
 *
 * @author  Peter Halasz
 */

/*
 * based on the original IntResourceForm.java
 *
 * Created on July 9, 2003, 10:21 PM
 *
 * another attempt at IntInputBox
 */

import java.awt.BorderLayout;
import java.awt.FlowLayout;
import java.awt.event.ActionEvent;
import java.awt.event.ActionListener;
import java.util.ArrayList;
import java.util.List;

import javax.swing.JButton;
import javax.swing.JFrame;
import javax.swing.JPanel;
import javax.swing.JScrollPane;
import javax.swing.JSplitPane;
import javax.swing.JTree;
import javax.swing.event.TreeSelectionEvent;
import javax.swing.event.TreeSelectionListener;
import javax.swing.tree.DefaultMutableTreeNode;

import net.pengo.resource.Resource;
import java.util.HashSet;
import java.util.Set;
/**
 *
 * @author  Peter Halasz
 */
abstract public class PropertiesForm extends JFrame
{
	
	private JSplitPane sp;
	private JTree nav;
	private JScrollPane scrollpane;
	private JPanel scrollcontents;
	private JButton apply;
	
	private boolean modded = false; // have changes been made?
	private boolean setup = false; // has the window been set up yet?
	private boolean saving = false; // in the middle of saving?
	private boolean waitingToUpdate = false;
	
	private Set modSet = new HashSet(); // pages that have been modded
	
	//String name;
	//private Data value;
	//private int size; // make dereferencable f(x)
	//int signed; ; // make dereferencable f(x)
	
	//private Editor;// = value+size, value, or this;
	
	/** Creates a new instance of IntResourceForm */
	//public IntResourceForm(OpenFile openFile) {
	//        this.openFile = openFile;
	//    }15
	
	
	public PropertiesForm()
	{
		super("Property editor!");
	}
	
	
	public void setVisible(boolean b)
	{
		if (!setup)
		{
			setupFrame();
		}
		
		super.setVisible(b);
	}
	
	public void show()
	{
		if (!setup)
		{
			setupFrame();
		}
		
		super.show();
	}
	
	public void setupFrame()
	{
		setup=true;
		JPanel buttons = new JPanel();
		buttons.setLayout(new FlowLayout(FlowLayout.RIGHT));
		JButton okay = new JButton("Okay");
		okay.addActionListener(new ActionListener()
							   {
					public void actionPerformed(ActionEvent e)
					{
						PropertiesForm.this.save();
						PropertiesForm.this.close();
					}
				});
		buttons.add(okay);
		
		JButton cancel = new JButton("Cancel");
		cancel.addActionListener(new ActionListener()
								 {
					public void actionPerformed(ActionEvent e)
					{
						PropertiesForm.this.close();
					}
				});
		buttons.add(cancel);
		
		apply = new JButton("Apply");
		apply.addActionListener(new ActionListener()
								{
					public void actionPerformed(ActionEvent e)
					{
						PropertiesForm.this.save();
					}
				});
		justSaved(); // grey Apply button
		buttons.add(apply);
		
		nav = getNav();
		scrollcontents = new JPanel(new BorderLayout());
		scrollpane = new JScrollPane(scrollcontents, JScrollPane.VERTICAL_SCROLLBAR_AS_NEEDED, JScrollPane.HORIZONTAL_SCROLLBAR_AS_NEEDED) ;
		sp = new JSplitPane(JSplitPane.HORIZONTAL_SPLIT, nav, scrollpane);
		
		getContentPane().add(sp);
		// apply button enabler
		getContentPane().add(buttons, BorderLayout.SOUTH);
		
		//FIXME: (default page) seems a bit dodgy but it works
		nav.setSelectionPath(nav.getPathForRow(-1));
		nav.setSelectionPath(nav.getPathForRow(0));
		//setPage((PropertyPage)nav.getPathForRow(0).getPath()[0]);
		
		//FIXME: magic numbers
		setSize(500,400);
		setLocation(20,20);
	}
	
	public boolean isSaving()
	{
		return saving;
	}
	
	/** getMenu and set page back to what it was. */
	protected JTree getNav()
	{
//		if (nav!=null)
//		return nav;
		
		nav = getMenu();
		
		nav.addTreeSelectionListener(new TreeSelectionListener()
									 {
					public void valueChanged(TreeSelectionEvent treeEvent)
					{
						DefaultMutableTreeNode node = (DefaultMutableTreeNode)nav.getLastSelectedPathComponent();
						if (node == null)
							return;
						
						Object userObj = node.getUserObject();
						
						if (userObj == null)
						{
							//System.out.println("nully wully: " + userObj);
							return;
						}
						else if (userObj instanceof PropertyPage)
						{
							//System.out.println("yay prop: " + userObj);
							PropertiesForm.this.setPage((PropertyPage)userObj);
						}
						else
						{
							//System.out.println("sumtingelse: " + userObj + "\n  type:" + node.getClass());
						}
					}
				});
		
		return nav;
	}
	
	public void close()
	{
		setVisible(false);
	}
	
	public void mod(PropertyPage p)
	{
		modded = true;
		//PropertiesForm.this.apply.setEnabled(true);
		apply.setEnabled(true);
		//System.out.println("mod list; added: " + p);
		modSet.add(p);
	}
	
	public void save()
	{
		setSaving(true);
		
		PropertyPage[] menu = (PropertyPage[]) modSet.toArray(new PropertyPage[0]);
		for (int i=0; i < menu.length; i++)
		{
			if (!menu[i].isValid())
			{
				sp.setRightComponent(menu[i]);
				System.out.println("This page is not valid.");
				return;
			}
		}
		
		for (int i=0; i < menu.length; i++)
		{
			menu[i].save();
		}
		
		// maybe need this.. maybe not?
		for (int i=0; i < menu.length; i++)
		{
			menu[i].build();
		}
		
		justSaved();
	}
	
	public void setSaving(boolean saving) {
		
		if (waitingToUpdate == true && this.saving == true && saving == false) {
			this.saving = saving;
			updateNav();
		}
		
		this.saving = saving;
	}
	
	public void justSaved()
	{
		modded = false;
		setSaving(false);
		apply.setEnabled(false);
		modSet.clear();
	}
	
	// sets which PropertyPage to display show
	public void setPage(PropertyPage page)
	{
//		FIXME
		scrollcontents.removeAll();
		scrollcontents.add(page, BorderLayout.CENTER);
		//sp.setRightComponent(scrollpane);
		//page.revalidate();
		scrollcontents.validate();
		scrollpane.validate();
		repaint();
		//scrollpane.revalidate();
		//pagevalidate();
	}
	
	/*
	 public ActionListener getModListener(){
	 return modListener;
	 }
	 */
	
	// override this please.
	abstract protected JTree getMenu();
	
	public void rebuild()
	{
		//FIXME: umm.. well..
		//sp.setLeftComponent(getNav());
		//System.out.println("rebuild...");
		
		repaint();
	}
	
	public void updateNav()
	{
		if (isSaving()) {
			waitingToUpdate = true;
			return;
		}
		
		//System.out.println("updating " + this);
		Resource subresSelected = null;
		if (nav != null)
		{
			//FIXME: need to re-select both the tree item, and opens its page
			//subresSelected = ((ResourceMultiPage)nav.getSelectionPath().getLastPathComponent()).getResource();
			//System.out.println(nav.getSelectionPath().getLastPathComponent().getClass() + ""); // javax.swing.tree.DefaultMutableTreeNode
		}
		nav = getNav();
		
		sp.setLeftComponent(nav);
		
		//FIXME: scan through nav and select subresSelected
		//for ()
	}
	
}

