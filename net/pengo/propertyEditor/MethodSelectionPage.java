/**
 * MethodSelection.java
 *
 * A way of selecting between multiple methods/classes of input for a properties page.
 * e.g. Combo box to select "Int" or "String" and pages for each type's input
 *
 * @author Created by Omnicore CodeGuide
 */

package net.pengo.propertyEditor;
import java.awt.BorderLayout;
import javax.swing.*;
import java.awt.event.ActionListener;
import java.awt.event.ActionEvent;



public class MethodSelectionPage extends EditablePage
{
	
	private PropertyPage[] page;
	private String name;
	private int selected = 0;
	private JComboBox selectBox;

	private JPanel main;
	
	public MethodSelectionPage(AbstractResourcePropertiesForm form, PropertyPage[] page, String name) {
		super(form);
		this.page = page;
		this.name = name;
		
		setLayout(new BorderLayout());
		
		selectBox = new JComboBox(page);
		selectBox.addActionListener( new ActionListener() {
				public void actionPerformed(ActionEvent e)
				{
					setSelected(MethodSelectionPage.this.selectBox.getSelectedIndex());
				}
			});
		selectBox.setSelectedIndex(selected);

		main = new JPanel(new BorderLayout());
		main.add(selectBox, BorderLayout.NORTH);
		add(main, BorderLayout.CENTER);
		
		build();
	}
	
	public PropertyPage getSelected() {
		return page[selected];
	}
	
	public void setSelected(int selected) {
		if (this.selected != selected) {
			main.remove(getSelected());
			this.selected = selected;
			form.mod();
			build();
		}
	}
	
	public void save() {
	    // save regardless of if it's modded
	    saveOp();
	}
    
    protected void saveOp()
	{
	    System.out.println("saving: " + getSelected());
	    getSelected().save();
	}
	
	public void buildOp()
	{
		getSelected().build();
		main.add(getSelected(), BorderLayout.CENTER);
		validate();
		repaint();
	}
	
	public String toString()
	{
		return name;
	}
	
	
	
}

